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
mod frame;
mod hover;
mod interaction;
mod long_lines;
mod navigation;
mod paint;
mod pipeline;
pub mod prepared;
mod prepared_update;
pub mod presentation;
mod search;
mod selection;
mod session;
mod session_interaction;
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
use quark_text::FontEpoch;
use quark_ui::FocusId;
use quark_ui::element::{ScrollHandle, ScrollbarVisibility};
use quark_ui::virtual_list::VariableList;

pub use annotations::{DiffAnchor, DiffAnnotation};
pub use hover::HoverHit;
pub use navigation::{DiffTarget, FileId, RevealAlign, Revision, SourcePoint};
pub use prepared::{AnnotationId, DiffPreviewLimit};
pub use prepared_update::{Preparation, PreparedSegment};
pub use search::{FindOptions, SearchCoverage, SearchDirection, SearchSides, SearchSummary};
pub use selection::CopyContent;
pub use session::DiffSessionViewState;
pub use session_interaction::{diff_session_view, diff_session_view_with};
pub use view::{diff_view, diff_view_custom, diff_view_with};

use annotations::AnnotationTable;
use prepared::{Metrics, RowPaint, ViewFrame};
use presentation::{DiffAppearance, DiffPresentation};
use search::SearchState;
use state::{FileMap, RowRefs, Segment};
use syntax::{DiffSyntax, SyntaxBudget};

/// Lines one click on an expand control reveals by default.
pub const REVEAL_STEP: u32 = diffy_diff::REVEAL_STEP;

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
    ToggleReflow {
        file: u32,
        line: u32,
    },
    JumpMove {
        file: u32,
        index: u32,
        here: bool,
    },
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
    Definition,
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
    Definition {
        point: navigation::SourcePoint,
        line: String,
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
    link: Option<(quark::selection::BlockKey, std::ops::Range<usize>)>,
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
            link: None,
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
}
