//! Moving through the diff: hunk and file steps, and revealing a semantic
//! target (a source line, a hunk, or a file), expanding collapsed context
//! when the target is hidden in it.
//!
//! Targets name files by [`FileId`]: in the static view the document's file
//! index, in a session view the app's id. Lines are zero-based source
//! lines, bytes UTF-8 offsets into the line; neither is a projection row.

use diffy_diff::{RowKind, Side};

use super::DiffViewState;
use super::state::{RowRef, source_line, store_index};

pub use diffy_diff::{FileId, Revision};

/// A position in a file's source: zero-based `line` on `side`, `byte` into
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourcePoint {
    pub file: FileId,
    pub side: Side,
    pub line: u32,
    pub byte: u32,
}

/// Where navigation can go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiffTarget {
    Source(SourcePoint),
    /// Hunk `hunk` of the file, counted from zero within it.
    Hunk {
        file: FileId,
        hunk: u32,
    },
    File(FileId),
}

/// Where a revealed target lands in the viewport.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum RevealAlign {
    Top,
    Center,
    /// Scroll only as far as needed to bring it into view.
    #[default]
    Nearest,
}

/// Lines of context kept above a target revealed inside a gap.
const REVEAL_CONTEXT: u32 = 3;

impl DiffViewState {
    /// The unit of `file`, when the view shows it.
    pub(crate) fn unit_of(&self, file: FileId) -> Option<u32> {
        if self.files.is_static() {
            return u32::try_from(file.0).ok();
        }
        self.files.id_slots.get(&file).copied()
    }

    /// The [`FileId`] of `unit`.
    pub(crate) fn file_id(&self, unit: u32) -> FileId {
        if self.files.is_static() {
            return FileId(u64::from(unit));
        }
        self.files.slot_ids[unit as usize]
    }

    /// Scrolls so projection row `row` is at the top (static view).
    pub fn scroll_to_row(&mut self, row: u32) {
        if let Some(index) = self.segments[0].list_rows.index(row) {
            let top = self.list.rows().offset_of_index(index as usize);
            self.set_scroll(top);
        }
    }

    /// Scrolls to list row `index` as `align` asks. Returns whether the
    /// view moved.
    pub(crate) fn scroll_to_index(&mut self, index: u32, align: RevealAlign) -> bool {
        let rows = self.list.rows();
        let top = rows.offset_of_index(index as usize);
        let height = rows.offset_of_index(index as usize + 1) - top;
        let (scroll, view) = (self.list.scroll_offset(), f64::from(self.viewport.1));
        let target = match align {
            RevealAlign::Top => top,
            RevealAlign::Center => top + height * 0.5 - view * 0.5,
            RevealAlign::Nearest if top < scroll => top,
            RevealAlign::Nearest if top + height > scroll + view => top + height - view,
            RevealAlign::Nearest => scroll,
        };
        self.set_scroll(target)
    }

    /// Scrolls to the next or previous hunk or file start: the header or
    /// gap row above a hunk's first line when there is one.
    pub(crate) fn jump(&mut self, forward: bool, hunks: bool) -> bool {
        let rows = self.list.rows();
        let current = self.list.scroll_offset();
        // List indices of every start, in display order.
        let starts = self.segments.iter().flat_map(|segment| {
            let p = &segment.projection;
            let starts = if hunks { &p.hunk_rows } else { &p.file_rows };
            starts.iter().filter_map(move |&start| {
                let above = start.checked_sub(1).filter(|&r| {
                    hunks && matches!(p.kind[r as usize], RowKind::HunkHeader | RowKind::Gap)
                });
                segment.list_rows.index(above.unwrap_or(start))
            })
        });
        // Half a point of slack: offsets are rounded to Fenwick units.
        let target = if forward {
            starts
                .clone()
                .find(|&i| rows.offset_of_index(i as usize) > current + 0.5)
        } else {
            starts
                .rev()
                .find(|&i| rows.offset_of_index(i as usize) < current - 0.5)
        };
        match target {
            Some(index) => {
                let top = rows.offset_of_index(index as usize);
                let moved = self.set_scroll(top);
                self.set_focus(Some(index));
                moved
            }
            None => false,
        }
    }

    pub fn focus_change(&mut self, forward: bool) -> Option<super::selection::LineHit> {
        let count = self.refs.len();
        let start = self.focused_index.map(|i| i as usize);
        let changed = |state: &Self, index: usize| match state.refs.get(index)? {
            RowRef::Line { seg, row } => {
                let p = &state.segments[seg as usize].projection;
                let kind = p.kind[row as usize];
                matches!(
                    kind,
                    RowKind::Added
                        | RowKind::Removed
                        | RowKind::Modified
                        | RowKind::Merged
                        | RowKind::Reflowed
                )
                .then_some((seg, row, kind))
            }
            _ => None,
        };
        let next = |index: usize| -> Option<usize> {
            if forward {
                (index + 1 < count).then_some(index + 1)
            } else {
                index.checked_sub(1)
            }
        };
        let mut index = match start {
            Some(i) => next(i)?,
            None if forward => 0,
            None => count.checked_sub(1)?,
        };
        let (seg, row, kind) = loop {
            if let Some(found) = changed(self, index) {
                break found;
            }
            index = next(index)?;
        };
        self.scroll_to_index(index as u32, RevealAlign::Nearest);
        self.set_focus(Some(index as u32));
        let side = if kind == RowKind::Removed {
            Side::Old
        } else {
            Side::New
        };
        let segment = &self.segments[seg as usize];
        let p = &segment.projection;
        let file = p.file[row as usize];
        let line = p.line(row, side)?;
        let line = source_line(&segment.doc, file, side, line);
        self.line_hit(segment.unit(file), side, line, kind)
    }

    /// Moves the keyboard focus target to list row `index`.
    pub(crate) fn set_focus(&mut self, index: Option<u32>) {
        let key = index.map(|i| self.ref_key(self.refs.at(i as usize)));
        self.focused_index = index;
        if key != self.focused {
            self.focused = key;
            self.revision += 1;
        }
    }

    /// The target the keyboard focus sits on: the focused row's first
    /// line, or its file.
    pub fn focused_target(&self) -> Option<DiffTarget> {
        self.focused?;
        self.target_of_index(self.focused_index?)
    }

    pub fn top_file(&self) -> Option<FileId> {
        let index = self.list.rows().row_at(self.list.scroll_offset())?;
        let unit = match self.refs.get(index)? {
            RowRef::Line { seg, row } => {
                let segment = &self.segments[seg as usize];
                segment.unit(segment.projection.file[row as usize])
            }
            RowRef::Fact { seg, file, .. } => self.segments[seg as usize].unit(file),
            RowRef::Pending { seg } => self.segments[seg as usize].unit(0),
            RowRef::Annotation { .. } | RowRef::More { .. } => return None,
        };
        Some(self.file_id(unit))
    }

    pub fn focused_hunk(&self) -> Option<(FileId, u32)> {
        self.focused?;
        let RowRef::Line { seg, row } = self.refs.get(self.focused_index? as usize)? else {
            return None;
        };
        let segment = &self.segments[seg as usize];
        let p = &segment.projection;
        let file = p.file[row as usize];
        let global = match p.kind[row as usize] {
            RowKind::Gap => p.gap(row)?.id.hunk?,
            RowKind::FileHeader => return None,
            _ => Some(p.hunk[row as usize]).filter(|&h| h != super::state::NONE)?,
        };
        let hunks = segment.doc.files().hunks[file as usize].clone();
        hunks
            .contains(&global)
            .then(|| (self.file_id(segment.unit(file)), global - hunks.start))
    }

    /// The source target a list row stands for.
    pub(crate) fn target_of_index(&self, index: u32) -> Option<DiffTarget> {
        let (seg, row) = match self.refs.get(index as usize)? {
            RowRef::Line { seg, row } => (seg as usize, row),
            RowRef::Fact { seg, file, .. } => {
                let unit = self.segments[seg as usize].unit(file);
                return Some(DiffTarget::File(self.file_id(unit)));
            }
            RowRef::Annotation { index } => {
                let a = &self.annotations.entry(index).annotation.anchor;
                return Some(DiffTarget::Source(SourcePoint {
                    file: a.file,
                    side: a.side,
                    line: a.lines.start,
                    byte: 0,
                }));
            }
            RowRef::More { .. } => return None,
            RowRef::Pending { seg } => {
                let unit = self.segments[seg as usize].unit(0);
                return Some(DiffTarget::File(self.file_id(unit)));
            }
        };
        let segment = &self.segments[seg];
        let p = &segment.projection;
        let file = p.file[row as usize];
        let id = self.file_id(segment.unit(file));
        let point = [Side::New, Side::Old].into_iter().find_map(|side| {
            let index = p.line(row, side)?;
            Some(SourcePoint {
                file: id,
                side,
                line: source_line(&segment.doc, file, side, index),
                byte: 0,
            })
        });
        Some(match point {
            Some(point) => DiffTarget::Source(point),
            None => DiffTarget::File(id),
        })
    }

    /// The first row a bounded preview leaves out, for its open control.
    pub(crate) fn preview_target(&self) -> Option<DiffTarget> {
        self.preview?;
        let hidden = self.segments.iter().enumerate().find_map(|(seg, segment)| {
            let row = segment.list_rows.first_hidden(segment.projection.len())?;
            Some((seg, row))
        });
        let (seg, row) = hidden?;
        let segment = &self.segments[seg];
        let p = &segment.projection;
        let file = p.file[row as usize];
        let id = self.file_id(segment.unit(file));
        let line = (row..p.len()).find_map(|r| {
            let file_here = p.file[r as usize];
            (file_here == file).then_some(())?;
            [Side::New, Side::Old].into_iter().find_map(|side| {
                let index = p.line(r, side)?;
                Some((side, source_line(&segment.doc, file, side, index)))
            })
        });
        Some(match line {
            Some((side, line)) => DiffTarget::Source(SourcePoint {
                file: id,
                side,
                line,
                byte: 0,
            }),
            None => DiffTarget::File(id),
        })
    }

    /// Scrolls `target` into view, expanding the collapsed context that
    /// hides a source line, and moves the keyboard focus to it. Returns
    /// whether the target exists.
    pub fn reveal_target(&mut self, target: DiffTarget, align: RevealAlign) -> bool {
        let Some(index) = self.target_index(target) else {
            return false;
        };
        self.scroll_to_index(index, align);
        self.set_focus(Some(index));
        true
    }

    /// The list row of `target`, expanding context to show a hidden line.
    fn target_index(&mut self, target: DiffTarget) -> Option<u32> {
        match target {
            DiffTarget::File(file) => {
                let (seg, file) = self.locate(self.unit_of(file)?)?;
                let segment = &self.segments[seg];
                let row = *segment.projection.file_rows.get(file as usize)?;
                segment.list_rows.index(row)
            }
            DiffTarget::Hunk { file, hunk } => {
                let unit = self.unit_of(file)?;
                self.unfold(unit);
                let (seg, file) = self.locate(unit)?;
                let segment = &self.segments[seg];
                let hunks = segment.doc.files().hunks[file as usize].clone();
                let global = hunks
                    .start
                    .checked_add(hunk)
                    .filter(|h| hunks.contains(h))?;
                let row = *segment.projection.hunk_rows.get(global as usize)?;
                segment.list_rows.index(row)
            }
            DiffTarget::Source(point) => {
                let unit = self.unit_of(point.file)?;
                self.unfold(unit);
                let (seg, file) = self.locate(unit)?;
                let index = store_index(&self.segments[seg].doc, file, point.side, point.line)?;
                self.reveal_line(seg, file, point.side, index);
                self.list_index_of_line(unit, point.side, index)
            }
        }
    }

    /// Unfolds a collapsed file, so a target inside it has rows.
    pub(crate) fn unfold(&mut self, unit: u32) {
        if self.collapsed.remove(&unit) {
            let anchor = self.anchor();
            self.rebuild_rows(anchor);
        }
    }

    /// Expands the gap hiding store line `index` of `file`'s `side`, from
    /// whichever end is nearer, keeping a few lines of context above it.
    /// Other expansion stays as it was.
    pub(crate) fn reveal_line(&mut self, seg: usize, file: u32, side: Side, index: u32) -> bool {
        let segment = &self.segments[seg];
        let p = &segment.projection;
        if p.row_of(file, side, index).is_some() {
            return false;
        }
        let gap = p.gaps.iter().copied().find(|g| {
            let start = match side {
                Side::Old => g.old_start,
                Side::New => g.new_start,
            };
            g.id.file == file && (start..start + g.hidden).contains(&index)
        });
        let Some(gap) = gap else {
            return false;
        };
        let start = match side {
            Side::Old => gap.old_start,
            Side::New => gap.new_start,
        };
        let from_top = index - start;
        let from_bottom = gap.hidden - from_top;
        let (reveal, amount) = if gap.id.hunk.is_none() || from_top < from_bottom {
            (diffy_diff::Reveal::Down, from_top + 1 + REVEAL_CONTEXT)
        } else {
            (diffy_diff::Reveal::Up, from_bottom + REVEAL_CONTEXT)
        };
        let unit = segment.unit(file);
        let id = diffy_diff::GapId {
            file: unit,
            ..gap.id
        };
        self.reveal(id, reveal, amount)
    }
}
