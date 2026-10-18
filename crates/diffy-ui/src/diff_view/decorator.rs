//! App-supplied content inside the diff view: header slots, annotation
//! rows, and the gutter utility of the focused row.
//!
//! The view positions and caches what a [`DiffDecorator`] returns; the app
//! owns what it shows and the actions its controls emit. The view never
//! learns what an annotation means (a comment thread, a diagnostic, a
//! review finding), and the diff crate implements no staging, review
//! scope, or undo.

use std::ops::Range;
use std::sync::Arc;

use diffy_diff::{DiffDocument, FileStatus, GapId, RowKind, Side};
use quark_ui::element::AnyElement;

use super::DiffViewState;
use super::navigation::{FileId, Revision};
use super::prepared::{AnnotationId, Columns};
use super::state::NONE;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileSource {
    pub id: FileId,
    pub revision: Revision,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HunkSource {
    pub file: FileSource,
    pub hunk: u32,
    pub old: Range<u32>,
    pub new: Range<u32>,
}

#[derive(Debug, Clone)]
struct UnitSource {
    source: FileSource,
    doc: Arc<DiffDocument>,
    file: u32,
}

#[derive(Debug, Clone, Default)]
pub struct DiffSources {
    units: Vec<Option<UnitSource>>,
}

impl DiffSources {
    pub fn file(&self, unit: u32) -> Option<FileSource> {
        Some(self.units.get(unit as usize)?.as_ref()?.source)
    }

    pub fn hunk(&self, unit: u32, global: u32) -> Option<HunkSource> {
        let unit = self.units.get(unit as usize)?.as_ref()?;
        let hunks = unit.doc.files().hunks[unit.file as usize].clone();
        if !hunks.contains(&global) {
            return None;
        }
        let h = unit.doc.hunks();
        let i = global as usize;
        let lines = |start: u32, len: u32| {
            let start = start.saturating_sub(1);
            start..start + len
        };
        Some(HunkSource {
            file: unit.source,
            hunk: global - hunks.start,
            old: lines(h.old_start[i], h.old_len[i]),
            new: lines(h.new_start[i], h.new_len[i]),
        })
    }

    pub fn global_hunk(&self, unit: u32, hunk: u32) -> Option<u32> {
        let entry = self.units.get(unit as usize)?.as_ref()?;
        let hunks = entry.doc.files().hunks[entry.file as usize].clone();
        let global = hunks.start.checked_add(hunk)?;
        hunks.contains(&global).then_some(global)
    }

    pub fn hunk_at(&self, unit: u32, hunk: u32) -> Option<HunkSource> {
        let entry = self.units.get(unit as usize)?.as_ref()?;
        let start = entry.doc.files().hunks[entry.file as usize].start;
        self.hunk(unit, start.checked_add(hunk)?)
    }

    pub fn hunk_of_line(&self, unit: u32, side: Side, line: u32) -> Option<HunkSource> {
        let entry = self.units.get(unit as usize)?.as_ref()?;
        entry.doc.files().hunks[entry.file as usize]
            .clone()
            .filter_map(|global| self.hunk(unit, global))
            .find(|hunk| {
                let range = match side {
                    Side::Old => &hunk.old,
                    Side::New => &hunk.new,
                };
                range.contains(&line)
            })
    }

    pub fn unit_of(&self, id: FileId) -> Option<u32> {
        self.units
            .iter()
            .position(|u| u.as_ref().is_some_and(|u| u.source.id == id))
            .map(|unit| unit as u32)
    }
}

impl DiffViewState {
    pub fn sources(&self) -> DiffSources {
        if self.files.is_static() {
            let Some(segment) = self.segments.first() else {
                return DiffSources::default();
            };
            let units = (0..segment.doc.file_count())
                .map(|file| {
                    Some(UnitSource {
                        source: FileSource {
                            id: FileId(u64::from(file)),
                            revision: Revision(segment.revision),
                        },
                        doc: segment.doc.clone(),
                        file,
                    })
                })
                .collect();
            return DiffSources { units };
        }
        let units = self
            .files
            .slot_seg
            .iter()
            .zip(&self.files.slot_ids)
            .map(|(&seg, &id)| {
                let segment = self.segments.get(seg as usize).filter(|_| seg != NONE)?;
                Some(UnitSource {
                    source: FileSource {
                        id,
                        revision: Revision(segment.revision),
                    },
                    doc: segment.doc.clone(),
                    file: 0,
                })
            })
            .collect();
        DiffSources { units }
    }
}

/// A press or drag over a line gutter, in window points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinePointer {
    pub x: f32,
    pub y: f32,
    /// Whether the pointer moved with the button held since the press.
    pub drag: bool,
}

#[derive(Clone, Default)]
pub struct DiffViewOptions {
    pub decorator: Option<std::rc::Rc<dyn DiffDecorator>>,
    pub sources: Option<std::rc::Rc<DiffSources>>,
    pub keys: Option<&'static [(&'static str, super::DiffKey)]>,
    /// Presses and drags over line gutters, for line selection.
    pub on_line: Option<fn(LinePointer) -> quark_ui::Action>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineMark {
    Selected,
}

#[derive(Debug, Clone, Copy)]
pub struct LineContext {
    pub file: u32,
    pub source: Option<FileSource>,
    pub side: Side,
    pub line: u32,
    pub kind: RowKind,
}

/// A place in a built-in file header that a decorator can fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HeaderSlot {
    /// Before the status and path, e.g. a file type icon.
    Prefix,
    /// After the path, e.g. a "best effort" or "loading" badge.
    Metadata,
    /// At the trailing edge, after the counts: the app's file actions.
    Actions,
    /// After the actions and a divider: the app's view controls.
    Trailing,
    /// After the path while the file is collapsed, saying why.
    Collapsed,
}

/// The file a header shows.
#[derive(Debug, Clone, Copy)]
pub struct HeaderContext<'a> {
    /// The document's file index.
    pub file: u32,
    pub source: Option<FileSource>,
    /// The path, or `old → new` for a rename or copy.
    pub title: &'a str,
    pub status: FileStatus,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
    /// The header row's size in points.
    pub width: f32,
    pub height: f32,
}

/// An annotation row the view has placed under its anchor.
#[derive(Debug, Clone, Copy)]
pub struct AnnotationContext<'a> {
    pub id: AnnotationId,
    pub side: Side,
    /// The anchored source lines, zero-based and half open.
    pub lines: &'a Range<u32>,
    /// The anchor's lines changed since it was attached.
    pub outdated: bool,
    /// The row's width in points; the content decides its height.
    pub width: f32,
}

/// A hunk header or collapsed gap row.
#[derive(Debug, Clone)]
pub struct SeparatorContext<'a> {
    pub file: u32,
    /// The gap the row folds, for [`super::DiffEvent::Expand`]; `None` for
    /// a hunk header.
    pub gap: Option<GapId>,
    pub source: Option<FileSource>,
    pub hunk: Option<HunkSource>,
    /// Lines the gap hides; zero for a hunk header.
    pub hidden: u32,
    /// The built-in row's text: the hidden-line count and the hunk header.
    pub title: &'a str,
    /// Where the gutters and text columns sit, so a fold bar can line up
    /// with the code.
    pub columns: Columns,
    /// The row's size in points.
    pub width: f32,
    pub height: f32,
}

/// The line row holding the keyboard focus, whose gutter offers the
/// decorator's utility (an Add comment button, say).
#[derive(Debug, Clone, Copy)]
pub struct GutterContext {
    pub file: u32,
    pub source: Option<FileSource>,
    pub side: Side,
    /// Zero-based source line on `side`.
    pub line: u32,
    pub kind: RowKind,
    /// The gutter's size in points.
    pub width: f32,
    pub height: f32,
}

/// Content the app places inside the view. Every method has a default
/// that adds nothing, so a decorator implements only the slots it fills.
///
/// The view caches what each slot returns and asks again only when the
/// row's inputs or [`Self::revision`] change, so `revision` must change
/// whenever any slot would return something different.
pub trait DiffDecorator {
    fn revision(&self) -> u64;

    /// Content for `slot` of a built-in header.
    fn header_slot(&self, _slot: HeaderSlot, _cx: &HeaderContext) -> Option<AnyElement> {
        None
    }

    /// A whole header under [`super::presentation::FileHeaders::Custom`];
    /// `None` falls back to the built-in one.
    fn header(&self, _cx: &HeaderContext) -> Option<AnyElement> {
        None
    }

    /// A whole hunk header or gap row; `None` keeps the built-in one. A
    /// gap's controls should emit [`super::DiffEvent::Expand`] for
    /// [`SeparatorContext::gap`].
    fn separator(&self, _cx: &SeparatorContext) -> Option<AnyElement> {
        None
    }

    /// The content of an annotation row, as wide as the view.
    fn annotation(&self, _cx: &AnnotationContext) -> Option<AnyElement> {
        None
    }

    /// A control drawn over the focused line row's gutter.
    fn gutter_utility(&self, _cx: &GutterContext) -> Option<AnyElement> {
        None
    }

    fn line_mark(&self, _cx: &LineContext) -> Option<LineMark> {
        None
    }

    /// The text of a header's state pill, such as "Modified · unstaged";
    /// `None` shows the file status alone.
    fn file_state(&self, _cx: &HeaderContext) -> Option<String> {
        None
    }

    /// The syntax scopes around a hunk, outermost first.
    fn hunk_scope(&self, _cx: &SeparatorContext) -> Option<Arc<[Arc<str>]>> {
        None
    }

    /// Actions at the trailing edge of a hunk header or gap row.
    fn hunk_actions(&self, _cx: &SeparatorContext) -> Option<AnyElement> {
        None
    }
}
