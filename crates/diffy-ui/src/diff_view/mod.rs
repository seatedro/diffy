//! A virtualized diff view over a [`diffy_diff::DiffDocument`]: unified or
//! side by side, with line numbers, word-level change highlights, syntax
//! colors, collapsible unchanged regions, file headers, selection and copy,
//! search, annotations, and keyboard navigation between hunks and files.
//!
//! The app owns a [`DiffViewState`]. Each frame it calls
//! [`DiffViewState::prepare`] with the frame's text system, which shapes
//! the rows entering the window (and nothing on a frame that repeats the
//! last one), then builds [`diff_view`]. Input comes back as
//! [`DiffEvent`]s for [`DiffViewState::handle`]. A diff whose files change
//! while it is shown uses [`DiffSessionViewState`] and
//! [`diff_session_view`] instead, with the same rendering and interaction.
//!
//! Rows live in a [`VariableList`] (a Fenwick row table), so a 100,000-line
//! diff builds only the rows on screen. Text is laid out by `quark-text`:
//! word wrap when [`DiffStyle::wrap`] is on, and hit-testing through the
//! shaped glyphs, so wide characters, emoji, and right-to-left text select
//! where they are drawn. Each text column scrolls sideways on its own
//! [`ScrollHandle`] when wrap is off.
//!
//! Selection endpoints are [`quark::selection::SelectionPoint`]s keyed by side, file, and
//! line, so a selection spans hunks and survives scrolling and expanding.
//! Copy takes the side under the selection when side by side; unified
//! copies [`DiffStyle::copy_side`] (new text by default). [`CopyContent`]
//! names every copy policy, including exact whole-file and whole-line
//! copies.
//!
//! Positions outside the view use [`SourcePoint`]s: a file, a side, a
//! zero-based source line, and a byte of that line. Search results,
//! annotation anchors, and navigation targets all convert through them.
//!
//! Syntax colors come from a `diffy-syntax` worker thread
//! ([`DiffViewState::enable_syntax`]); rows repaint as files finish. A
//! session view highlights each file revision as it arrives, on one
//! worker, and keeps the colors of sides an update did not change.

mod annotations;
pub mod decorator;
mod long_lines;
mod navigation;
mod paint;
pub mod prepared;
pub mod presentation;
mod search;
mod selection;
mod session;
mod state;
pub mod syntax;
mod view;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use diffy_diff::{
    ComparisonOptions, ContextPolicy, DiffDocument, DiffLimits, GapId, InlineOptions, Mode,
    Projection, Reveal, Side,
};
use diffy_syntax::{GrammarStore, HighlightWorker};
use quark::selection::Selection;
use quark_render::scene::Rect;
use quark_text::{FontEpoch, LayoutCache, TextSystem};
use quark_ui::FocusId;
use quark_ui::element::{ScrollHandle, ScrollbarVisibility, WHEEL_LINE_PX};
use quark_ui::virtual_list::VariableList;

pub use annotations::{DiffAnchor, DiffAnnotation};
pub use navigation::{DiffTarget, FileId, RevealAlign, Revision, SourcePoint};
pub use prepared::{AnnotationId, DiffPreviewLimit};
pub use search::{FindOptions, SearchCoverage, SearchDirection, SearchSides, SearchSummary};
pub use selection::CopyContent;
pub use session::{DiffSessionViewState, diff_session_view, diff_session_view_with};
pub use view::{diff_view, diff_view_with};

use annotations::AnnotationTable;
use prepared::{Metrics, RowPaint, ViewFrame};
use presentation::{DiffAppearance, DiffLayout, DiffPresentation};
use search::SearchState;
use state::{FileMap, RowRefs, Segment};
use syntax::{DiffSyntax, SyntaxBudget};

/// Lines one click on an expand control reveals by default.
pub const REVEAL_STEP: u32 = diffy_diff::REVEAL_STEP;

/// Autoscroll speed per point the pointer is past the edge band, in
/// points per millisecond.
const AUTOSCROLL_GAIN: f32 = 0.02;
/// Fastest autoscroll, in points per millisecond.
const AUTOSCROLL_MAX: f32 = 4.0;
/// Longest frame gap autoscroll integrates over, so a stalled frame does
/// not jump the view.
const AUTOSCROLL_MAX_DT_MS: u64 = 50;
/// Frame interval the view asks for while a drag autoscrolls.
pub(crate) const AUTOSCROLL_FRAME_MS: u64 = 16;

/// Which text a unified selection copies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum CopySide {
    /// Context and added lines: the new file's text.
    #[default]
    New,
    /// Context and removed lines: the old file's text.
    Old,
    /// Every selected line as shown.
    Shown,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiffStyle {
    /// Monospace text size in points.
    pub font_size: f32,
    /// Line height as a multiple of the font size.
    pub line_height: f32,
    /// Wrap long lines at word boundaries instead of scrolling sideways.
    pub wrap: bool,
    pub copy_side: CopySide,
}

impl Default for DiffStyle {
    fn default() -> Self {
        Self {
            font_size: 13.0,
            line_height: 1.5,
            wrap: false,
            copy_side: CopySide::New,
        }
    }
}

/// Input from [`diff_view`], in window coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DiffEvent {
    Press {
        x: f32,
        y: f32,
    },
    Drag {
        x: f32,
        y: f32,
    },
    Release,
    Scroll(i32),
    ScrollTo(f32),
    Key(DiffKey),
    Expand(GapId, Reveal),
    /// The open control of a bounded preview's last row.
    OpenFull,
    /// The add-annotation control of a line (gutter utility or focused
    /// row command): the row's [`prepared::RowPaint::file`], the side, and
    /// the zero-based source line from [`prepared::RowPaint::source_lines`].
    Annotate {
        file: u32,
        side: Side,
        line: u32,
    },
    /// The collapse control of a file header: the header's
    /// [`prepared::RowPaint::file`].
    ToggleFile {
        file: u32,
    },
    /// An annotation row measured its content at the row's width.
    AnnotationMeasured {
        id: AnnotationId,
        revision: u64,
        height: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiffKey {
    NextHunk,
    PrevHunk,
    NextFile,
    PrevFile,
    LineUp,
    LineDown,
    PageUp,
    PageDown,
    Home,
    End,
    Copy,
    SelectAll,
    /// Next and previous search match.
    NextMatch,
    PrevMatch,
    /// Annotate the selection, or the focused row's line.
    Annotate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffOutcome {
    Unchanged,
    Changed,
    /// Copy was pressed: put this on the clipboard.
    Copy(String),
    /// The preview's open control was pressed: show the full diff at
    /// `target`, the first row the preview left out.
    OpenFull {
        target: DiffTarget,
    },
    /// The app should open its annotation editor for `anchor`.
    Annotate {
        anchor: DiffAnchor,
    },
}

/// A selection drag in progress.
#[derive(Debug, Clone, Copy)]
struct Drag {
    /// The pointer, view-local.
    pointer: (f32, f32),
    /// Clock of the last autoscroll step, while past an edge.
    last_ms: Option<u64>,
}

/// What the last prepare materialized from.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PrepareKey {
    window: (usize, usize),
    scroll: u64,
    revision: u64,
    scale: u32,
    /// Each column's sideways scroll in long-line window grid steps.
    hgrid: [u32; 2],
}

/// App-owned diff view state. See the [module docs](self).
pub struct DiffViewState {
    id: &'static str,
    label: &'static str,
    focus: FocusId,
    /// The static view has one segment holding the whole document; a
    /// session view has one per file, in display order.
    segments: Vec<Segment>,
    files: FileMap,
    mode: Mode,
    /// What each row of `list` shows.
    refs: RowRefs,
    /// List index of each annotation entry's row, or [`state::NONE`].
    annotation_rows: Vec<u32>,
    /// Keyed by list index; see [`Self::rebuild_rows`].
    list: VariableList,
    style: DiffStyle,
    viewport: (f32, f32),
    hscroll: [ScrollHandle; 2],
    /// The vertical scrollbar shows only on demand, as `scrollbar`
    /// decides.
    scrollbar_auto_hide: bool,
    scrollbar: ScrollbarVisibility,
    selection: Option<Selection>,
    /// The side a side-by-side selection copies.
    selection_side: Side,
    drag: Option<Drag>,
    /// The view's bounds in the window as of the last frame.
    bounds: Rc<Cell<Rect>>,
    /// Measured metrics, with the scale factor's bits and the fonts they
    /// were measured under.
    metrics: Option<(Metrics, (u32, FontEpoch))>,
    painted: HashMap<u64, Rc<RowPaint>>,
    frame: Option<Rc<ViewFrame>>,
    prepared: Option<PrepareKey>,
    content_w: [f64; 2],
    syntax: DiffSyntax,
    long_lines: long_lines::LongLines,
    /// Session views: one bridge per file slot, all on `session_worker`.
    slot_syntax: Vec<DiffSyntax>,
    session_worker: Option<(HighlightWorker, GrammarStore)>,
    syntax_wake: Option<Arc<dyn Fn() + Send + Sync>>,
    presentation: DiffPresentation,
    appearance: DiffAppearance,
    preview: Option<DiffPreviewLimit>,
    context_policy: ContextPolicy,
    comparison: ComparisonOptions,
    inline_options: InlineOptions,
    limits: DiffLimits,
    search: SearchState,
    annotations: AnnotationTable,
    /// Units of files folded under their headers.
    collapsed: HashSet<u32>,
    /// Key of the row the last navigation moved the keyboard focus to.
    focused: Option<u64>,
    /// List index of that row, kept with the rows.
    focused_index: Option<u32>,
    /// Source of segment generations, so a stamp never matches a row of a
    /// replaced document.
    generations: u64,
    revision: u64,
    /// Bumped with every frame `prepare` builds.
    frame_id: u64,
}

impl DiffViewState {
    /// `id` names cache entries and accessibility ids (unique in the
    /// window); `focus` is the view's keyboard focus target.
    pub fn new(id: &'static str, focus: FocusId, doc: DiffDocument) -> Self {
        let mut state = Self::empty(id, focus);
        state.segments.push(Segment::new(
            Arc::new(doc),
            0,
            0,
            Mode::Unified,
            ContextPolicy::default(),
            ComparisonOptions::default(),
        ));
        state
            .syntax
            .reset(state.segments[0].doc.file_count() as usize);
        state.syntax.set_budget(SyntaxBudget {
            side_bytes: state.limits.syntax_file_bytes,
        });
        state.rebuild_rows(None);
        state
    }

    /// A view with no segments, for the session view to fill.
    fn empty(id: &'static str, focus: FocusId) -> Self {
        Self {
            id,
            label: "Diff",
            focus,
            segments: Vec::new(),
            files: FileMap::default(),
            mode: Mode::Unified,
            refs: RowRefs::default(),
            annotation_rows: Vec::new(),
            list: VariableList::new(1.0, 0.0),
            style: DiffStyle::default(),
            viewport: (0.0, 0.0),
            hscroll: [ScrollHandle::new(), ScrollHandle::new()],
            scrollbar_auto_hide: false,
            scrollbar: ScrollbarVisibility::new(),
            selection: None,
            selection_side: Side::New,
            drag: None,
            bounds: Rc::default(),
            metrics: None,
            painted: HashMap::new(),
            frame: None,
            prepared: None,
            content_w: [0.0; 2],
            syntax: DiffSyntax::default(),
            long_lines: Default::default(),
            slot_syntax: Vec::new(),
            session_worker: None,
            syntax_wake: None,
            presentation: DiffPresentation::default(),
            appearance: DiffAppearance::default(),
            preview: None,
            context_policy: ContextPolicy::default(),
            comparison: ComparisonOptions::default(),
            inline_options: InlineOptions::default(),
            limits: DiffLimits::default(),
            search: SearchState::default(),
            annotations: AnnotationTable::default(),
            focused: None,
            focused_index: None,
            collapsed: HashSet::new(),
            generations: 0,
            revision: 0,
            frame_id: 0,
        }
    }

    pub fn with_label(mut self, label: &'static str) -> Self {
        self.label = label;
        self
    }

    pub fn with_style(mut self, style: DiffStyle) -> Self {
        self.set_style(style);
        self
    }

    /// Show the vertical scrollbar only while the pointer is over the view,
    /// its thumb is held, or briefly after the view scrolls or gains focus
    /// (see [`ScrollbarVisibility`]). Without it that scrollbar always
    /// shows.
    /// The sideways scrollbars of unwrapped lines always auto-hide.
    pub fn with_scrollbar_auto_hide(mut self) -> Self {
        self.scrollbar_auto_hide = true;
        self
    }

    // ---- Queries -------------------------------------------------------

    pub fn document(&self) -> &DiffDocument {
        &self.segments[0].doc
    }

    pub fn projection(&self) -> &Projection {
        &self.segments[0].projection
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn style(&self) -> DiffStyle {
        self.style
    }

    pub fn presentation(&self) -> DiffPresentation {
        self.presentation
    }

    pub fn limits(&self) -> DiffLimits {
        self.limits
    }

    pub fn comparison(&self) -> ComparisonOptions {
        self.comparison
    }

    pub fn inline_options(&self) -> InlineOptions {
        self.inline_options
    }

    /// Changed line pairs of `file` the whitespace policy shows as
    /// unchanged, for a "N whitespace-only changes hidden" status.
    pub fn hidden_whitespace_changes(&self, file: FileId) -> u32 {
        self.unit_of(file)
            .and_then(|unit| self.locate(unit))
            .map_or(0, |(seg, file)| {
                self.segments[seg]
                    .comparison
                    .hidden_whitespace_changes(file)
            })
    }

    /// The vertical scroll offset; [`ViewFrame::scroll`] has it exactly.
    pub fn scroll_offset(&self) -> f32 {
        self.list.scroll_offset() as f32
    }

    /// Height of every row together: what a bounded preview needs to show
    /// without scrolling.
    pub fn content_height(&self) -> f32 {
        self.list.rows().total_extent() as f32
    }

    pub fn selection(&self) -> Option<Selection> {
        self.selection
    }

    /// The frame the last [`Self::prepare`] built, as the renderer reads
    /// it.
    pub fn frame(&self) -> Option<&Rc<ViewFrame>> {
        self.frame.as_ref()
    }

    /// Rows the last [`Self::prepare`] materialized: the window plus
    /// overscan.
    pub fn window(&self) -> Range<u32> {
        let range = self.list.window(self.overscan()).range;
        range.start as u32..range.end as u32
    }

    /// The projection row at the top of the viewport. A metadata or
    /// annotation row counts as the row it belongs to.
    pub fn top_row(&self) -> Option<u32> {
        let index = self.list.rows().row_at(self.list.scroll_offset())?;
        self.projection_row_of(index)
    }

    pub fn horizontal_scroll(&self, side: Side) -> &ScrollHandle {
        &self.hscroll[side as usize]
    }

    // ---- Changes -------------------------------------------------------

    /// Shows another document; expansion, selection, search position, and
    /// scroll reset. Annotations of the previous document become outdated.
    pub fn set_document(&mut self, doc: DiffDocument) {
        self.generations += 1;
        let revision = self.segments[0].revision + 1;
        let mut segment = Segment::new(
            Arc::new(doc),
            0,
            self.generations,
            self.mode,
            self.context_policy,
            self.comparison,
        );
        segment.revision = revision;
        self.segments[0] = segment;
        self.selection = None;
        self.focused = None;
        self.focused_index = None;
        self.painted.clear();
        self.content_w = [0.0; 2];
        self.syntax
            .reset(self.segments[0].doc.file_count() as usize);
        self.annotations.outdate_all();
        self.rerun_search();
        self.rebuild_rows(None);
        self.syntax
            .request(&self.segments[0].doc, self.segments[0].generation);
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.presentation.layout = match mode {
            Mode::Unified => DiffLayout::Unified,
            Mode::Split => DiffLayout::Split,
        };
        self.switch_mode(mode);
    }

    /// Changes the comparison layout, keeping the top source line in place.
    fn switch_mode(&mut self, mode: Mode) {
        if mode == self.mode {
            return;
        }
        let anchor = self.anchor();
        self.mode = mode;
        for segment in &mut self.segments {
            segment.rebuild(mode);
        }
        self.rebuild_rows(anchor);
    }

    pub fn set_style(&mut self, style: DiffStyle) {
        if style == self.style {
            return;
        }
        let anchor = self.anchor();
        self.style = style;
        self.metrics = None;
        self.painted.clear();
        self.content_w = [0.0; 2];
        self.rebuild_rows(anchor);
    }

    /// Changes how the diff is drawn; see [`DiffPresentation`]. An explicit
    /// layout switches the mode as [`Self::set_mode`] does; an automatic one
    /// is resolved against the viewport on the next [`Self::prepare`].
    pub fn set_presentation(&mut self, presentation: DiffPresentation) {
        if presentation == self.presentation {
            return;
        }
        let columns_moved = (presentation.numbers, presentation.markers)
            != (self.presentation.numbers, self.presentation.markers);
        let bands_moved = (presentation.separators, presentation.headers)
            != (self.presentation.separators, self.presentation.headers);
        self.presentation = presentation;
        match presentation.layout {
            DiffLayout::Unified => self.switch_mode(Mode::Unified),
            DiffLayout::Split => self.switch_mode(Mode::Split),
            DiffLayout::Auto { .. } => {}
        }
        if columns_moved && self.style.wrap {
            // Wrapped rows were measured for the old text width.
            self.painted.clear();
        }
        if bands_moved {
            // Header and separator rows change height.
            let anchor = self.anchor();
            self.rebuild_rows(anchor);
        }
        self.revision += 1;
    }

    /// How far each reveal step goes and how few hidden lines a gap may
    /// keep; applies to every file, keeping what is revealed.
    pub fn set_context_policy(&mut self, policy: ContextPolicy) {
        let anchor = self.anchor();
        let mode = self.mode;
        for segment in &mut self.segments {
            segment.expansion.set_policy(policy);
            segment.rebuild(mode);
        }
        self.context_policy = policy;
        self.rebuild_rows(anchor);
    }

    /// Collapses a gap's revealed lines again. Returns whether any were
    /// revealed.
    pub fn collapse(&mut self, gap: GapId) -> bool {
        let Some((seg, file)) = self.locate(gap.file) else {
            return false;
        };
        if !self.segments[seg].expansion.collapse(GapId { file, ..gap }) {
            return false;
        }
        let anchor = self.anchor();
        let mode = self.mode;
        self.segments[seg].rebuild(mode);
        self.rebuild_rows(anchor);
        true
    }

    /// Reveals every unchanged line of every file.
    pub fn expand_all(&mut self) -> bool {
        let anchor = self.anchor();
        let mode = self.mode;
        let mut changed = false;
        for segment in &mut self.segments {
            if segment.expansion.reveal_all(&segment.doc) {
                segment.rebuild(mode);
                changed = true;
            }
        }
        if changed {
            self.rebuild_rows(anchor);
        }
        changed
    }

    /// How removed and added lines are paired and which whitespace changes
    /// are hidden; see [`ComparisonOptions`]. Every file is compared again
    /// and the top source line stays in place. Copy, search, and patch
    /// export keep reading the exact sources.
    pub fn set_comparison(&mut self, options: ComparisonOptions) {
        if options == self.comparison {
            return;
        }
        let anchor = self.anchor();
        let mode = self.mode;
        for segment in &mut self.segments {
            segment.compare(mode, options);
        }
        self.comparison = options;
        // Rows may pair with other lines now.
        self.painted.clear();
        self.rebuild_rows(anchor);
    }

    /// Which changed units inside a line pair are highlighted, and the
    /// limits past which none are. The shorter of
    /// [`InlineOptions::max_line_bytes`] and
    /// [`DiffLimits::inline_line_bytes`] applies.
    pub fn set_inline_options(&mut self, options: InlineOptions) {
        if options == self.inline_options {
            return;
        }
        self.inline_options = options;
        for segment in &mut self.segments {
            segment.clear_inline();
        }
        self.painted.clear();
        self.revision += 1;
    }

    /// Local color overrides over the theme's.
    pub fn set_appearance(&mut self, appearance: DiffAppearance) {
        if appearance != self.appearance {
            self.appearance = appearance;
            self.revision += 1;
        }
    }

    /// Bounds the view to a compact preview of at most `limit.max_rows`
    /// rows, ending in a row that counts the rest and offers
    /// [`DiffOutcome::OpenFull`]; `None` shows every row.
    pub fn set_preview_limit(&mut self, limit: Option<DiffPreviewLimit>) {
        if limit != self.preview {
            self.preview = limit;
            self.rebuild_rows(None);
        }
    }

    pub fn set_limits(&mut self, limits: DiffLimits) {
        if limits != self.limits {
            self.limits = limits;
            let budget = SyntaxBudget {
                side_bytes: limits.syntax_file_bytes,
            };
            self.syntax.set_budget(budget);
            for bridge in &mut self.slot_syntax {
                bridge.set_budget(budget);
            }
            // Word diffs were computed under the old line limit.
            for segment in &mut self.segments {
                segment.clear_inline();
            }
            self.painted.clear();
            self.content_w = [0.0; 2];
            self.rerun_search();
            self.revision += 1;
        }
    }

    /// The view's size in points.
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        if self.viewport != (width, height) {
            if self.style.wrap && self.viewport.0 != width {
                self.painted.clear();
            }
            self.viewport = (width.max(0.0), height.max(0.0));
            self.list.set_viewport_height(self.viewport.1);
            self.revision += 1;
        }
    }

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
    fn poll_syntax(&mut self) -> bool {
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

    /// Folds a file's rows under its header, or unfolds them. Returns
    /// whether that changed anything.
    pub fn set_file_collapsed(&mut self, file: FileId, collapsed: bool) -> bool {
        let Some(unit) = self.unit_of(file).filter(|&u| self.locate(u).is_some()) else {
            return false;
        };
        let changed = if collapsed {
            self.collapsed.insert(unit)
        } else {
            self.collapsed.remove(&unit)
        };
        if changed {
            let anchor = self.anchor();
            self.rebuild_rows(anchor);
        }
        changed
    }

    pub fn is_file_collapsed(&self, file: FileId) -> bool {
        self.unit_of(file)
            .is_some_and(|unit| self.collapsed.contains(&unit))
    }

    pub fn set_selection(&mut self, selection: Option<Selection>, side: Side) {
        self.selection = selection;
        self.selection_side = side;
        self.revision += 1;
    }

    /// Reveals lines of a collapsed gap; see [`diffy_diff::Expansion::reveal`].
    /// The gap's `file` is the view's file index (the document's, for the
    /// static view).
    pub fn reveal(&mut self, gap: GapId, reveal: Reveal, amount: u32) -> bool {
        let Some((seg, file)) = self.locate(gap.file) else {
            return false;
        };
        let local = GapId { file, ..gap };
        let segment = &mut self.segments[seg];
        if !segment
            .expansion
            .reveal(&segment.doc, local, reveal, amount)
        {
            return false;
        }
        let anchor = self.anchor();
        let mode = self.mode;
        self.segments[seg].rebuild(mode);
        self.rebuild_rows(anchor);
        true
    }

    fn set_scroll(&mut self, offset: f64) -> bool {
        let before = self.list.scroll_offset();
        if self.list.set_scroll_offset(offset) == before {
            return false;
        }
        self.revision += 1;
        true
    }

    // ---- Input ---------------------------------------------------------

    /// Applies an event from [`diff_view`].
    pub fn handle(&mut self, event: DiffEvent) -> DiffOutcome {
        let changed = match event {
            DiffEvent::Press { x, y } => {
                let (x, y) = self.local(x, y);
                self.selection_side = self.side_at(x);
                self.drag = Some(Drag {
                    pointer: (x, y),
                    last_ms: None,
                });
                self.selection = self.point_at(x, y).map(Selection::collapsed);
                self.revision += 1;
                true
            }
            DiffEvent::Drag { x, y } => {
                let (x, y) = self.local(x, y);
                if let Some(drag) = &mut self.drag {
                    drag.pointer = (x, y);
                }
                match (self.selection, self.point_at(x, y)) {
                    (Some(s), Some(point)) if s.focus != point => {
                        self.selection = Some(Selection::new(s.anchor, point));
                        self.revision += 1;
                        true
                    }
                    _ => false,
                }
            }
            DiffEvent::Release => {
                self.drag = None;
                false
            }
            DiffEvent::Scroll(lines) => self.set_scroll(
                self.list.scroll_offset() + f64::from(lines) * f64::from(WHEEL_LINE_PX),
            ),
            DiffEvent::ScrollTo(offset) => self.set_scroll(f64::from(offset)),
            DiffEvent::Expand(gap, reveal) => {
                self.reveal(gap, reveal, self.context_policy.reveal_step)
            }
            DiffEvent::Key(key) => return self.key(key),
            DiffEvent::OpenFull => {
                return match self.preview_target() {
                    Some(target) => DiffOutcome::OpenFull { target },
                    None => DiffOutcome::Unchanged,
                };
            }
            DiffEvent::Annotate { file, side, line } => {
                let file = self.file_id(file);
                return match self.anchor_for(file, side, line..line + 1) {
                    Some(anchor) => DiffOutcome::Annotate { anchor },
                    None => DiffOutcome::Unchanged,
                };
            }
            DiffEvent::ToggleFile { file } => {
                let collapsed = self.collapsed.contains(&file);
                let id = self.file_id(file);
                self.set_file_collapsed(id, !collapsed)
            }
            DiffEvent::AnnotationMeasured {
                id,
                revision,
                height,
            } => self.set_annotation_height(id, revision, height),
        };
        if changed {
            DiffOutcome::Changed
        } else {
            DiffOutcome::Unchanged
        }
    }

    fn key(&mut self, key: DiffKey) -> DiffOutcome {
        let line = f64::from(self.metrics().line_h);
        let page = (f64::from(self.viewport.1) - line * 2.0).max(line);
        let offset = self.list.scroll_offset();
        let moved = match key {
            DiffKey::NextHunk => self.jump(true, true),
            DiffKey::PrevHunk => self.jump(false, true),
            DiffKey::NextFile => self.jump(true, false),
            DiffKey::PrevFile => self.jump(false, false),
            DiffKey::LineUp => self.set_scroll(offset - line),
            DiffKey::LineDown => self.set_scroll(offset + line),
            DiffKey::PageUp => self.set_scroll(offset - page),
            DiffKey::PageDown => self.set_scroll(offset + page),
            DiffKey::Home => self.set_scroll(0.0),
            DiffKey::End => self.set_scroll(f64::MAX),
            DiffKey::Copy => return DiffOutcome::Copy(self.selected_text()),
            DiffKey::SelectAll => {
                self.select_all();
                true
            }
            DiffKey::NextMatch => self.next_match(SearchDirection::Forward).is_some(),
            DiffKey::PrevMatch => self.next_match(SearchDirection::Backward).is_some(),
            DiffKey::Annotate => {
                return match self.annotate_selection_or_focus() {
                    Some(anchor) => DiffOutcome::Annotate { anchor },
                    None => DiffOutcome::Unchanged,
                };
            }
        };
        if moved {
            DiffOutcome::Changed
        } else {
            DiffOutcome::Unchanged
        }
    }

    // ---- Autoscroll ----------------------------------------------------

    /// A drag is held past the top or bottom edge: the view scrolls each
    /// frame, so it needs another one.
    pub fn wants_frame(&self) -> bool {
        self.drag
            .is_some_and(|drag| self.autoscroll_velocity(drag.pointer.1) != 0.0)
    }

    /// Signed autoscroll speed in points per millisecond for a pointer at
    /// view-local `y`: zero away from the edges, growing with the distance
    /// into or past a one-line band at the top or bottom.
    fn autoscroll_velocity(&self, y: f32) -> f32 {
        let (edge, height) = (self.metrics().line_h, self.viewport.1);
        let past = if y < edge {
            y - edge
        } else if y > height - edge {
            y - (height - edge)
        } else {
            0.0
        };
        (past * AUTOSCROLL_GAIN).clamp(-AUTOSCROLL_MAX, AUTOSCROLL_MAX)
    }

    /// Scroll a held drag past an edge by the time since its last step.
    /// Returns whether the view moved.
    fn autoscroll(&mut self, now_ms: u64) -> bool {
        let Some(mut drag) = self.drag else {
            return false;
        };
        let velocity = self.autoscroll_velocity(drag.pointer.1);
        let mut moved = false;
        if velocity == 0.0 {
            drag.last_ms = None;
        } else {
            if let Some(last) = drag.last_ms {
                let dt = now_ms.saturating_sub(last).min(AUTOSCROLL_MAX_DT_MS);
                let offset = self.list.scroll_offset() + f64::from(velocity) * dt as f64;
                moved = self.set_scroll(offset);
            }
            drag.last_ms = Some(now_ms);
        }
        self.drag = Some(drag);
        moved
    }

    /// Move the selection's focus to whatever now sits under the held
    /// pointer. Returns whether it changed.
    fn follow_pointer(&mut self) -> bool {
        let (Some(drag), Some(selection)) = (self.drag, self.selection) else {
            return false;
        };
        match self.point_at(drag.pointer.0, drag.pointer.1) {
            Some(point) if point != selection.focus => {
                self.selection = Some(Selection::new(selection.anchor, point));
                self.revision += 1;
                true
            }
            _ => false,
        }
    }

    /// Advances a drag's autoscroll to `now_ms`, shapes the rows entering
    /// the window, and builds the frame the view paints. Does nothing (and
    /// allocates nothing) when the window, the state, and the scale are
    /// what the last call saw. `scale` must be the frame's scale factor.
    pub fn prepare(
        &mut self,
        text: &mut TextSystem,
        layouts: &mut LayoutCache,
        scale: f32,
        now_ms: u64,
    ) {
        if self.poll_syntax() {
            self.revision += 1;
        }
        if self.long_lines.poll() {
            self.revision += 1;
        }
        let scrolled = self.autoscroll(now_ms);
        self.build_frame(text, layouts, scale);
        // Rows moved under the held pointer: its selection end follows.
        if scrolled && self.follow_pointer() {
            self.build_frame(text, layouts, scale);
        }
    }
}
