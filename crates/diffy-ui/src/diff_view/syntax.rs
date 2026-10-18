//! Syntax coordination for the diff view: one highlight request per file
//! side to a `diffy-syntax` worker, the spans and [`SyntaxStatus`] each
//! side holds, and the per-file generation row stamps hash so rows repaint
//! as files finish.
//!
//! This is the bridge between the view state and the syntax layer. The
//! state calls only the methods below; everything else here may change
//! without touching the state.
//!
//! What is highlighted depends on how much of a file the document holds:
//!
//! - Whole files (a diff of two texts, or a patch hydrated with its exact
//!   sources) are highlighted per side as one document each, so a string or
//!   block comment opened before a hunk, or between two hunks, colors the
//!   lines it really covers. The status is [`SyntaxStatus::Ready`].
//! - A parsed patch holds only its hunks. Each hunk side is highlighted on
//!   its own, so a comment left open at the end of one hunk cannot color
//!   the next, whose surroundings are unknown. Colors inside a hunk are
//!   best effort, which [`SyntaxStatus::PartialSource`] reports; hydrate
//!   the file for exact colors.
//!
//! Each side picks its language from its own path (a rename from `.js` to
//! `.ts` highlights the old side as JavaScript), unless
//! [`DiffSyntax::set_language`] overrides it. Sides larger than the
//! [`SyntaxBudget`] (64 MiB unless lowered) are not parsed
//! ([`SyntaxStatus::Limited`]).
//!
//! A whole side longer than a couple of mebibytes is highlighted window by
//! window on the worker, with bounded memory. The lines around its focus
//! ([`DiffSyntax::set_focus`], the top visible line) are colored first, on
//! their own, then exact windows arrive in order from the start; the side
//! reports [`SyntaxStatus::Streaming`] with its progress until the last
//! one lands.
//!
//! Every request carries a generation unique to this bridge, and a side
//! keeps a result only when it answers that side's newest request with a
//! newer revision (grammars that arrive late recolor a request with higher
//! revisions). A result for a replaced source is dropped, so stale colors
//! never land on newer text.
//!
//! Coordinates: `file` is the document's file index, `side` the
//! [`Side`], and every range is a half-open UTF-8 byte range of that
//! side's [`diffy_diff::TextStore`]. Layout spans returned by
//! [`DiffSyntax::spans`] are relative to the start of the requested range.
//!
//! Errors: files without a known language, empty sides, and files out of
//! range are skipped; results for another request, an unknown file, or an
//! older revision than the one held are dropped. Nothing here panics on
//! stale input.

use std::ops::Range;
use std::sync::Arc;

use diffy_diff::{DiffDocument, Side, SourceCoverage, TextStore};
use diffy_syntax::{
    GrammarStore, HighlightFocus, HighlightKind, HighlightRequest, HighlightSpan, HighlightWorker,
    Highlighted, LanguageId, LanguageStatus, Priority,
};
use quark_text::TextSpan;

use super::DiffViewState;

/// How far a file side's colors can be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyntaxStatus {
    /// Requested, or waiting for its grammar to arrive; plain until then.
    Pending,
    /// A long side arriving window by window: exact colors cover the first
    /// `permille` thousandths of it, and the lines around its focus may be
    /// colored ahead of them.
    Streaming { permille: u16 },
    /// Highlighted from the whole file.
    Ready,
    /// No grammar for the side's language (or its path names none); plain.
    MissingGrammar,
    /// Highlighted hunk by hunk from a patch without the whole file:
    /// best effort, since lexical state before each hunk is unknown.
    PartialSource,
    /// Larger than the [`SyntaxBudget`]; plain.
    Limited,
}

impl SyntaxStatus {
    /// A short description for a file header or accessibility status.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pending | Self::Streaming { .. } => "Highlighting",
            Self::Ready => "Highlighted",
            Self::MissingGrammar => "No grammar, plain text",
            Self::PartialSource => "Best-effort colors, partial source",
            Self::Limited => "Too large to highlight",
        }
    }

    /// How much of the side has exact colors, 0 to 1, while it streams.
    pub fn progress(self) -> Option<f32> {
        match self {
            Self::Streaming { permille } => Some(f32::from(permille) / 1000.0),
            _ => None,
        }
    }

    /// The status a file header shows for a file's two sides: the least
    /// trustworthy of them.
    fn worst(self, other: Self) -> Self {
        let rank = |status| match status {
            Self::Ready => 0,
            Self::Streaming { permille } => 1 + u32::from(1000 - permille.min(1000)),
            Self::Pending => 1002,
            Self::PartialSource => 1003,
            Self::MissingGrammar => 1004,
            Self::Limited => 1005,
        };
        if rank(other) > rank(self) {
            other
        } else {
            self
        }
    }
}

/// Bounds on what the bridge parses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SyntaxBudget {
    /// Bytes of one side past which it stays plain instead of starting a
    /// parse. Matches `diffy_diff::DiffLimits::syntax_file_bytes`.
    pub side_bytes: usize,
}

impl Default for SyntaxBudget {
    fn default() -> Self {
        Self {
            side_bytes: diffy_diff::DETAIL_SIDE_BYTES,
        }
    }
}

/// What one file side asked the worker for and holds.
#[derive(Default)]
struct SideSyntax {
    /// Set by [`DiffSyntax::set_language`]; wins over the path's.
    language_override: Option<LanguageId>,
    /// The language the side's path names.
    path_language: Option<LanguageId>,
    /// The language it is highlighted as.
    language: Option<LanguageId>,
    source: Option<Arc<str>>,
    fragments: Option<Arc<[Range<u32>]>>,
    /// Generation of the newest request, 0 for none.
    request: u64,
    /// Whether a result for `request` has arrived.
    answered: bool,
    priority: Priority,
    status: Option<SyntaxStatus>,
    spans: Option<Arc<[HighlightSpan]>>,
    revision: Option<u32>,
    /// Where a streamed highlight colors first; shared with the worker.
    focus: HighlightFocus,
    /// A streamed side's exact parts, in order from byte 0 without gaps,
    /// each with its start; `exact_end` is where the last one ends.
    parts: Vec<(u32, Arc<[HighlightSpan]>)>,
    exact_end: u32,
    /// Its focus colored on its own, ahead of the exact parts.
    inexact: Option<(Range<u32>, Arc<[HighlightSpan]>)>,
}

impl SideSyntax {
    /// Whether highlighting `source` as `language` in `fragments` is what
    /// this side already asked for.
    fn asked(
        &self,
        language: &Option<LanguageId>,
        source: &Arc<str>,
        fragments: &Option<Arc<[Range<u32>]>>,
    ) -> bool {
        self.language == *language
            && self.source.as_ref().is_some_and(|s| Arc::ptr_eq(s, source))
            && self.fragments == *fragments
    }
}

/// Highlights held for one document. See the [module docs](self).
#[derive(Default)]
pub struct DiffSyntax {
    worker: Option<HighlightWorker>,
    store: GrammarStore,
    budget: SyntaxBudget,
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
    sides: Vec<[SideSyntax; 2]>,
    /// Bumped per file each time one of its sides recolors or changes
    /// status.
    generations: Vec<u32>,
    /// The document generation of the last [`Self::request`].
    document: u64,
    /// Last request generation handed out.
    requests: u64,
    /// Files on screen; their requests run first. `None` is every file.
    visible: Option<Range<u32>>,
}

impl DiffSyntax {
    /// Starts highlighting on a background thread of its own with
    /// `store`'s grammars.
    pub fn enable(&mut self, store: GrammarStore) {
        self.attach(HighlightWorker::new(store.clone()), store);
    }

    /// Starts highlighting on `worker`'s thread, shared with whoever else
    /// uses it, through a handle of this bridge's own. `store` must be the
    /// one `worker` was made with; it answers which languages have no
    /// grammar.
    pub fn enable_shared(&mut self, worker: &HighlightWorker, store: GrammarStore) {
        self.attach(worker.share(), store);
    }

    /// Whether a worker is attached.
    pub fn is_enabled(&self) -> bool {
        self.worker.is_some()
    }

    fn attach(&mut self, worker: HighlightWorker, store: GrammarStore) {
        if let Some(wake) = &self.wake {
            let wake = wake.clone();
            worker.set_wake(move || wake());
        }
        self.worker = Some(worker);
        self.store = store;
        // Requests made to an earlier worker are lost with it, so the next
        // `request` asks again.
        for side in self.sides.iter_mut().flatten() {
            side.source = None;
            side.request = 0;
        }
    }

    /// Calls `wake` on the worker thread whenever a result is ready to
    /// [`Self::poll`], so an idle app can wake its event loop instead of
    /// polling every frame.
    pub fn set_wake(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        if let Some(worker) = &self.worker {
            let wake = wake.clone();
            worker.set_wake(move || wake());
        }
        self.wake = Some(wake);
    }

    /// Bounds what is parsed; takes effect at the next [`Self::request`].
    pub fn set_budget(&mut self, budget: SyntaxBudget) {
        self.budget = budget;
    }

    /// Forgets every held highlight, for a document of `files` files.
    pub fn reset(&mut self, files: usize) {
        self.sides = std::iter::repeat_with(Default::default)
            .take(files)
            .collect();
        self.generations = vec![0; files];
    }

    /// Asks the worker for every side of `doc` (the document of
    /// `generation`) whose language is known, unless the side already
    /// asked for the same source, language, and coverage: an unchanged
    /// side keeps its colors. A side whose source changed drops its old
    /// colors at once, since their offsets belong to the old text.
    pub fn request(&mut self, doc: &DiffDocument, generation: u64) {
        self.document = generation;
        if self.sides.len() != doc.file_count() as usize {
            self.reset(doc.file_count() as usize);
        }
        if self.worker.is_none() {
            return;
        }
        for file in 0..doc.file_count() {
            for side in [Side::Old, Side::New] {
                let store = doc.text(file, side);
                let fragments = (doc.coverage(file) == SourceCoverage::PatchOnly)
                    .then(|| hunk_fragments(doc, file, side, store));
                let path = side_path(doc, file, side);
                self.request_side(file, side, path, store.shared(), fragments);
            }
        }
    }

    /// Highlights `file`'s `side` as `language` (or by its path again for
    /// `None`), re-requesting it when that changes its language.
    pub fn set_language(&mut self, file: u32, side: Side, language: Option<LanguageId>) {
        let Some(held) = self
            .sides
            .get_mut(file as usize)
            .map(|s| &mut s[side as usize])
        else {
            return;
        };
        if held.language_override == language {
            return;
        }
        held.language_override = language;
        let (Some(source), fragments) = (held.source.clone(), held.fragments.clone()) else {
            return;
        };
        let path_language = held.path_language.clone();
        self.request_resolved(file, side, path_language, &source, fragments);
    }

    fn request_side(
        &mut self,
        file: u32,
        side: Side,
        path: &str,
        source: &Arc<str>,
        fragments: Option<Arc<[Range<u32>]>>,
    ) {
        self.request_resolved(file, side, LanguageId::from_path(path), source, fragments);
    }

    /// Requests a side whose path names `path_language`.
    fn request_resolved(
        &mut self,
        file: u32,
        side: Side,
        path_language: Option<LanguageId>,
        source: &Arc<str>,
        fragments: Option<Arc<[Range<u32>]>>,
    ) {
        let visible = self.is_visible(file);
        let (Some(worker), Some(held)) = (
            &self.worker,
            self.sides
                .get_mut(file as usize)
                .map(|s| &mut s[side as usize]),
        ) else {
            return;
        };
        let language = held.language_override.clone().or(path_language.clone());
        held.path_language = path_language;
        if held.asked(&language, source, &fragments) {
            return;
        }
        let status = if source.is_empty() {
            None
        } else if language.is_none() {
            Some(SyntaxStatus::MissingGrammar)
        } else if source.len() > self.budget.side_bytes {
            Some(SyntaxStatus::Limited)
        } else {
            Some(SyntaxStatus::Pending)
        };
        held.language = language.clone();
        held.source = Some(source.clone());
        held.fragments = fragments.clone();
        held.status = status;
        held.spans = None;
        held.parts.clear();
        held.exact_end = 0;
        held.inexact = None;
        held.revision = None;
        held.answered = false;
        held.request = 0;
        self.generations[file as usize] += 1;
        let (Some(SyntaxStatus::Pending), Some(language)) = (status, language) else {
            return;
        };
        self.requests += 1;
        held.request = self.requests;
        held.priority = if visible {
            Priority::Visible
        } else {
            Priority::Background
        };
        let mut request = HighlightRequest::new(language, source.clone())
            .priority(held.priority)
            .focus(held.focus.clone());
        if let Some(fragments) = fragments {
            request = request.fragments(fragments);
        }
        worker.request_with(slot(file, side), held.request, request);
    }

    /// The files on screen, whose requests the worker takes before the
    /// rest. Cheap to call every frame: it does nothing unless they
    /// changed.
    pub fn set_visible_files(&mut self, files: Range<u32>) {
        if self.visible.as_ref() == Some(&files) {
            return;
        }
        self.visible = Some(files);
        let Some(worker) = &self.worker else {
            return;
        };
        for (file, sides) in (0u32..).zip(&mut self.sides) {
            let priority = if self.visible.as_ref().is_none_or(|v| v.contains(&file)) {
                Priority::Visible
            } else {
                Priority::Background
            };
            for (side, held) in [Side::Old, Side::New].into_iter().zip(sides.iter_mut()) {
                if held.request != 0 && !held.answered && held.priority != priority {
                    held.priority = priority;
                    worker.prioritize(slot(file, side), priority);
                }
            }
        }
    }

    /// The byte of `file`'s `side` the reader is at (the top visible line),
    /// which a long side still streaming colors first. Cheap to call every
    /// frame.
    pub fn set_focus(&self, file: u32, side: Side, byte: usize) {
        if let Some(held) = self.sides.get(file as usize).map(|s| &s[side as usize]) {
            held.focus.set(Some(byte));
        }
    }

    fn is_visible(&self, file: u32) -> bool {
        self.visible.as_ref().is_none_or(|v| v.contains(&file))
    }

    /// Takes every finished result. Returns whether any recolored a file.
    pub fn poll(&mut self, generation: u64) -> bool {
        let Some(worker) = &self.worker else {
            return false;
        };
        let mut results = Vec::new();
        while let Ok(Some(done)) = worker.try_recv() {
            results.push(done);
        }
        let mut changed = false;
        for done in results {
            changed |= self.take(done, generation);
        }
        changed
    }

    /// Blocks until every side with a request has its first result (all of
    /// its exact parts, when it streams), then takes it. For tests and
    /// screenshots, which need the colored state at once; a side whose
    /// grammar is still arriving counts as answered once its plain result
    /// is in.
    pub fn finish_pending(&mut self) -> bool {
        let mut changed = self.poll(self.document);
        loop {
            let waiting = self.sides.iter().flatten().any(|s| {
                s.request != 0
                    && (!s.answered || matches!(s.status, Some(SyntaxStatus::Streaming { .. })))
            });
            let Some(worker) = self.worker.as_ref().filter(|_| waiting) else {
                return changed;
            };
            match worker.recv() {
                Ok(done) => changed |= self.take(done, self.document),
                Err(_) => return changed,
            }
        }
    }

    /// Keeps a result when it answers its side's newest request (of the
    /// document of `generation`) with a newer revision than the one held.
    /// Returns whether it was kept.
    pub fn take(&mut self, done: Highlighted, generation: u64) -> bool {
        if generation != self.document {
            return false;
        }
        let (file, side) = ((done.slot / 2) as usize, (done.slot % 2) as usize);
        let Some(held) = self.sides.get_mut(file).map(|s| &mut s[side]) else {
            return false;
        };
        if held.request == 0
            || done.generation != held.request
            || held.revision.is_some_and(|r| r >= done.revision)
        {
            return false;
        }
        let Some(language) = held.language.as_ref() else {
            return false;
        };
        held.answered = true;
        held.revision = Some(done.revision);
        let len = held.source.as_ref().map_or(0, |s| s.len());
        let streaming = match &done.part {
            None => {
                held.spans = Some(done.spans.into());
                held.parts.clear();
                held.inexact = None;
                false
            }
            Some(part) if part.exact => {
                if part.range.start == 0 {
                    // A new pass (a grammar arrived) replaces the old one.
                    held.parts.clear();
                } else if part.range.start != held.exact_end {
                    return false;
                }
                held.parts.push((part.range.start, done.spans.into()));
                held.exact_end = part.range.end;
                held.spans = None;
                (part.range.end as usize) < len
            }
            Some(part) => {
                held.inexact = Some((part.range.clone(), done.spans.into()));
                (held.exact_end as usize) < len
            }
        };
        held.status = Some(if done.unresolved.contains(language) {
            SyntaxStatus::Pending
        } else if self.store.status(language) == LanguageStatus::Unavailable {
            // Resolved by the worker already, so this is a map lookup.
            SyntaxStatus::MissingGrammar
        } else if streaming {
            let permille = (u64::from(held.exact_end) * 1000 / len.max(1) as u64) as u16;
            SyntaxStatus::Streaming { permille }
        } else if held.fragments.is_some() {
            SyntaxStatus::PartialSource
        } else {
            SyntaxStatus::Ready
        });
        if !streaming {
            held.inexact = None;
        }
        self.generations[file] += 1;
        true
    }

    /// How many times `file` has recolored or changed status, for row
    /// stamps.
    pub fn file_generation(&self, file: u32) -> Option<u32> {
        self.generations.get(file as usize).copied()
    }

    /// How far `file`'s `side` colors can be trusted; `None` without a
    /// worker, for an empty or missing side, or for a file out of range.
    pub fn status(&self, file: u32, side: Side) -> Option<SyntaxStatus> {
        self.worker.as_ref()?;
        self.sides.get(file as usize)?[side as usize].status
    }

    /// The status a file header shows: the less trustworthy of its sides.
    pub fn file_status(&self, file: u32) -> Option<SyntaxStatus> {
        let old = self.status(file, Side::Old);
        let new = self.status(file, Side::New);
        match (old, new) {
            (Some(a), Some(b)) => Some(a.worst(b)),
            (a, b) => a.or(b),
        }
    }

    /// Layout spans splitting `range` of a store at highlight boundaries,
    /// with each span's kind. Empty when the side holds no highlight.
    pub fn spans(
        &self,
        file: u32,
        side: Side,
        range: Range<usize>,
    ) -> (Vec<TextSpan>, Arc<[HighlightKind]>) {
        let Some(held) = self.sides.get(file as usize).map(|h| &h[side as usize]) else {
            return (Vec::new(), Arc::from([]));
        };
        let mut out = Vec::new();
        let mut tones = Vec::new();
        let base = range.start;
        let mut push = |from: usize, to: usize, kind: HighlightKind| {
            if from < to {
                out.push(TextSpan {
                    range: from - base..to - base,
                    weight: None,
                    style: None,
                    kind: None,
                    size: None,
                    letter_spacing: None,
                    keep_together: false,
                });
                tones.push(kind);
            }
        };
        let mut at = range.start;
        let mut colored = false;
        // Sorted, disjoint span lists covering `range` in order: the whole
        // result, or the exact parts it touches, then the inexact window
        // past them.
        let exact_end = held.exact_end as usize;
        let first_part = held
            .parts
            .partition_point(|(start, _)| (*start as usize) <= range.start)
            .saturating_sub(1);
        // Each list with where it starts counting.
        let lists = held
            .spans
            .iter()
            .map(|spans| (spans, 0))
            .chain(
                held.parts[first_part..]
                    .iter()
                    .map(|(start, spans)| (spans, *start as usize)),
            )
            .chain(
                held.inexact
                    .iter()
                    .map(|(r, spans)| (spans, exact_end.max(r.start as usize))),
            );
        for (spans, start) in lists {
            let from = range.start.max(start);
            if from >= range.end {
                break;
            }
            let first = spans.partition_point(|s| s.range().end <= from);
            for span in &spans[first..] {
                let r = span.range();
                if r.start >= range.end {
                    break;
                }
                let (from, to) = (r.start.max(from).max(at), r.end.min(range.end));
                if from >= to {
                    continue;
                }
                push(at, from, HighlightKind::Normal);
                push(from, to, span.kind);
                at = to;
                colored = true;
            }
        }
        if colored {
            push(at, range.end, HighlightKind::Normal);
        }
        (out, tones.into())
    }
}

fn slot(file: u32, side: Side) -> u64 {
    u64::from(file) * 2 + side as u64
}

/// The path that names `side`'s language: its own, or the other side's
/// when it has none (the old side of an added file).
fn side_path(doc: &DiffDocument, file: u32, side: Side) -> &str {
    let meta = &doc.files().meta[file as usize];
    let (own, other) = match side {
        Side::Old => (&meta.old_path, &meta.new_path),
        Side::New => (&meta.new_path, &meta.old_path),
    };
    own.as_deref().or(other.as_deref()).unwrap_or("")
}

/// Byte ranges of `store` that each hunk of `file` shows on `side`, in
/// order, each with its final newline. Hunks with no lines on the side are
/// skipped.
fn hunk_fragments(
    doc: &DiffDocument,
    file: u32,
    side: Side,
    store: &TextStore,
) -> Arc<[Range<u32>]> {
    let blocks = doc.blocks();
    let (starts, lens) = match side {
        Side::Old => (&blocks.old_store, &blocks.old_len),
        Side::New => (&blocks.new_store, &blocks.new_len),
    };
    let hunks = doc.files().hunks[file as usize].clone();
    let text_len = store.as_str().len();
    let line_start = |line: u32| store.line_range(line).map_or(text_len, |range| range.start);
    hunks
        .filter_map(|hunk| {
            let blocks = doc.hunks().blocks[hunk as usize].clone();
            if blocks.is_empty() {
                return None;
            }
            let (first, last) = (blocks.start as usize, blocks.end as usize - 1);
            let (from, to) = (starts[first], starts[last] + lens[last]);
            (to > from).then(|| line_start(from) as u32..line_start(to) as u32)
        })
        .collect()
}

impl DiffViewState {
    /// Highlights every file on a background thread, by file extension,
    /// with `store`'s grammars. Files whose language has no grammar (or
    /// whose grammar is still downloading) stay plain until it arrives.
    /// A session view also highlights each file revision as it arrives.
    pub fn enable_syntax(&mut self, store: GrammarStore) {
        if !self.files.is_static() {
            self.attach_session_syntax(HighlightWorker::new(store.clone()), store);
            return;
        }
        self.syntax.enable(store);
        self.syntax
            .request(&self.segments[0].doc, self.segments[0].generation);
    }

    /// [`Self::enable_syntax`] on a worker shared with other views, so many
    /// diffs need one parsing thread.
    pub fn enable_syntax_shared(&mut self, worker: &HighlightWorker, store: GrammarStore) {
        if !self.files.is_static() {
            self.attach_session_syntax(worker.share(), store);
            return;
        }
        self.syntax.enable_shared(worker, store);
        self.syntax
            .request(&self.segments[0].doc, self.segments[0].generation);
    }

    /// Calls `wake` from the syntax worker when results are ready, so an
    /// idle app can ask for a frame instead of polling.
    pub fn set_syntax_wake(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let each = |bridge: &mut DiffSyntax| {
            let wake = wake.clone();
            bridge.set_wake(move || wake());
        };
        each(&mut self.syntax);
        self.slot_syntax.iter_mut().for_each(each);
        self.syntax_wake = Some(wake);
    }

    /// Session views: highlights every file on `worker`, one bridge per
    /// file slot, each a handle of its own on the worker's thread.
    fn attach_session_syntax(&mut self, worker: HighlightWorker, store: GrammarStore) {
        // Bridges of an earlier worker lose their requests with it.
        self.slot_syntax.clear();
        self.session_worker = Some((worker, store));
        for seg in 0..self.segments.len() {
            self.request_slot_syntax(seg);
        }
    }

    /// Asks for the colors of segment `seg`'s current document. Sides
    /// whose source did not change keep the colors they have.
    pub(crate) fn request_slot_syntax(&mut self, seg: usize) {
        let Some((worker, store)) = &self.session_worker else {
            return;
        };
        let segment = &self.segments[seg];
        let slot = segment.slot as usize;
        if self.slot_syntax.len() <= slot {
            self.slot_syntax.resize_with(slot + 1, DiffSyntax::default);
        }
        let bridge = &mut self.slot_syntax[slot];
        if !bridge.is_enabled() {
            if let Some(wake) = &self.syntax_wake {
                let wake = wake.clone();
                bridge.set_wake(move || wake());
            }
            bridge.set_budget(SyntaxBudget {
                side_bytes: self.limits.syntax_file_bytes,
            });
            bridge.enable_shared(worker, store.clone());
        }
        bridge.request(&segment.doc, segment.generation);
    }

    /// Drops the colors of a slot whose file left the session.
    pub(crate) fn drop_slot_syntax(&mut self, slot: u32) {
        if let Some(bridge) = self.slot_syntax.get_mut(slot as usize) {
            *bridge = DiffSyntax::default();
        }
    }

    /// The bridge holding segment `seg`'s colors, and the index it knows
    /// the segment's `file` by.
    pub(crate) fn syntax_of(&self, seg: usize, file: u32) -> Option<(&DiffSyntax, u32)> {
        if self.files.is_static() {
            return Some((&self.syntax, file));
        }
        let slot = self.segments.get(seg)?.slot;
        self.slot_syntax.get(slot as usize).map(|s| (s, file))
    }

    /// Takes finished highlights. Returns whether any recolored a file.
    pub(super) fn poll_syntax(&mut self) -> bool {
        if self.files.is_static() {
            return self
                .segments
                .first()
                .is_some_and(|segment| self.syntax.poll(segment.generation));
        }
        let mut changed = false;
        for segment in &self.segments {
            if let Some(bridge) = self.slot_syntax.get_mut(segment.slot as usize) {
                changed |= bridge.poll(segment.generation);
            }
        }
        changed
    }

    /// Blocks until every requested highlight has its first result and
    /// takes it, so the next frame shows colors. For tests and
    /// screenshots.
    pub fn finish_syntax(&mut self) -> bool {
        let mut changed = self.syntax.finish_pending();
        for bridge in &mut self.slot_syntax {
            changed |= bridge.finish_pending();
        }
        if changed {
            self.revision += 1;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = "\
--- a/lib.rs
+++ b/lib.rs
@@ -1,2 +1,2 @@
 /* one
-a
+b
@@ -10,2 +10,2 @@
 fn c() {}
-d
+e
";

    // Catches fragment ranges that run into the next hunk or drop a
    // hunk's final line: each hunk side covers exactly its own lines.
    #[test]
    fn patch_hunks_become_one_fragment_per_side() {
        let doc = diffy_diff::parse_unified(PATCH).unwrap();
        let ranges = |side| {
            let store = doc.text(0, side);
            hunk_fragments(&doc, 0, side, store)
                .iter()
                .map(|r| &store.as_str()[r.start as usize..r.end as usize])
                .collect::<Vec<_>>()
        };

        assert_eq!(
            (ranges(Side::Old), ranges(Side::New)),
            (
                vec!["/* one\na\n", "fn c() {}\nd\n"],
                vec!["/* one\nb\n", "fn c() {}\ne\n"]
            )
        );
    }

    // Catches progressive highlights being dropped or stale ones winning:
    // later revisions of a side's newest request recolor it, while earlier
    // revisions and results of another request do not.
    #[test]
    fn results_replace_only_when_newer_for_the_newest_request() {
        let doc = diffy_diff::parse_unified(PATCH).unwrap();
        let mut syntax = DiffSyntax::default();
        // Requests go nowhere; results are fed in by hand.
        syntax.attach(HighlightWorker::gone(), GrammarStore::none());
        syntax.request(&doc, 1);
        // The old side asked first (request 1), then the new side (2).
        let slot = slot(0, Side::New);
        let mut held = Vec::new();
        // (request generation, revision, highlighted length)
        for (generation, revision, length) in
            [(2, 0, 2), (2, 1, 5), (2, 0, 1), (1, 2, 9), (2, 1, 7)]
        {
            syntax.take(
                Highlighted {
                    slot,
                    generation,
                    revision,
                    source: doc.text(0, Side::New).shared().clone(),
                    spans: vec![HighlightSpan {
                        offset: 0,
                        length,
                        kind: HighlightKind::Keyword,
                    }],
                    pending: false,
                    unresolved: Vec::new(),
                    part: None,
                },
                1,
            );
            let (spans, _) = syntax.spans(0, Side::New, 0..9);
            held.push(spans.first().map(|span| span.range.clone()));
        }

        assert_eq!(
            held,
            [Some(0..2), Some(0..5), Some(0..5), Some(0..5), Some(0..5)]
        );
    }
}
