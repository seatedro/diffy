//! Annotation slots: full-width rows the app fills (comments, findings,
//! hunk actions), anchored to source lines of one file revision.
//!
//! The view positions and measures the rows; the app owns their content,
//! replies, resolution, and persistence. An annotation sits below the row
//! of its anchor's last line, on the anchor's side. Inside collapsed
//! context it adds to the gap's count instead. An anchor whose revision is
//! not the file's current one, or whose lines no longer exist, is
//! outdated: it shows under its file's header and never attaches to
//! another line on its own. A session update carries anchors over only
//! where its remap proves the lines unchanged.
//!
//! Heights start as an estimate and become exact when the renderer
//! reports a measurement ([`super::DiffEvent::AnnotationMeasured`]); the
//! code above and the top of the viewport stay put while they change.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use diffy_diff::Side;
use quark_ui::virtual_list::RowKey;

use super::DiffViewState;
use super::navigation::{DiffTarget, FileId, Revision};
use super::prepared::{AnnotationId, AnnotationSlot, Metrics};
use super::selection::decode_key;
use super::state::{Segment, source_line, store_index};

/// Source lines of one file revision.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiffAnchor {
    pub file: FileId,
    pub revision: Revision,
    pub side: Side,
    /// Zero-based source lines, half open.
    pub lines: Range<u32>,
}

/// An app annotation: where it attaches and its content revision, which
/// the app bumps when the content (and so its height) changes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiffAnnotation {
    pub id: AnnotationId,
    pub anchor: DiffAnchor,
    pub revision: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub annotation: DiffAnnotation,
    pub outdated: bool,
    /// The last measured height and the content revision it was measured
    /// for.
    pub measured: Option<(u64, f32)>,
}

impl Hash for Entry {
    // What the row's paint shows; its height is the list's business.
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.annotation.hash(state);
        self.outdated.hash(state);
    }
}

#[derive(Debug, Default)]
pub(crate) struct AnnotationTable {
    entries: Vec<Entry>,
}

impl AnnotationTable {
    pub fn entry(&self, index: u32) -> &Entry {
        &self.entries[index as usize]
    }

    pub fn entries_mut(&mut self) -> impl Iterator<Item = &mut Entry> {
        self.entries.iter_mut()
    }

    pub fn outdate_all(&mut self) {
        for entry in &mut self.entries {
            entry.outdated = true;
        }
    }
}

/// Where each annotation goes in the row table.
#[derive(Debug, Default)]
pub(crate) struct Placement {
    /// Below the row of `(unit, side, store index)`.
    pub at_line: HashMap<(u32, Side, u32), Vec<u32>>,
    /// Under the file header of the unit.
    pub outdated: HashMap<u32, Vec<u32>>,
}

/// Estimated height of an unmeasured annotation, in lines.
const ESTIMATED_LINES: f32 = 3.0;

impl DiffViewState {
    /// Replaces the annotations. Heights measured for an id at the same
    /// content revision are kept.
    pub fn set_annotations(&mut self, annotations: Vec<DiffAnnotation>) {
        let measured: HashMap<AnnotationId, (u64, f32)> = self
            .annotations
            .entries
            .iter()
            .filter_map(|e| Some((e.annotation.id, e.measured?)))
            .collect();
        self.annotations.entries = annotations
            .into_iter()
            .map(|annotation| {
                let outdated = !self.anchor_is_current(&annotation.anchor);
                let measured = measured
                    .get(&annotation.id)
                    .copied()
                    .filter(|&(revision, _)| revision == annotation.revision);
                Entry {
                    annotation,
                    outdated,
                    measured,
                }
            })
            .collect();
        let anchor = self.anchor();
        self.rebuild_rows(anchor);
    }

    /// Each annotation with whether it is outdated, in the order given.
    pub fn annotations(&self) -> impl Iterator<Item = (&DiffAnnotation, bool)> {
        self.annotations
            .entries
            .iter()
            .map(|e| (&e.annotation, e.outdated))
    }

    /// Marks each annotation of `file` outdated unless its anchor names
    /// existing lines of the file's current revision.
    pub(crate) fn recheck_annotations(&mut self, file: FileId) {
        let flags: Vec<Option<bool>> = self
            .annotations
            .entries
            .iter()
            .map(|e| {
                let a = &e.annotation.anchor;
                (a.file == file).then(|| !self.anchor_is_current(a))
            })
            .collect();
        for (entry, outdated) in self.annotations.entries.iter_mut().zip(flags) {
            if let Some(outdated) = outdated {
                entry.outdated = outdated;
            }
        }
    }

    /// Whether `anchor` names existing lines of its file's current
    /// revision.
    fn anchor_is_current(&self, anchor: &DiffAnchor) -> bool {
        let Some((seg, file)) = self.unit_of(anchor.file).and_then(|u| self.locate(u)) else {
            return false;
        };
        let segment = &self.segments[seg];
        let last = anchor.lines.end.max(anchor.lines.start + 1) - 1;
        anchor.revision == Revision(segment.revision)
            && store_index(&segment.doc, file, anchor.side, anchor.lines.start).is_some()
            && store_index(&segment.doc, file, anchor.side, last).is_some()
    }

    /// Records an annotation's measured height. Rows above it and the top
    /// of the viewport stay in place. Returns whether anything changed.
    pub fn set_annotation_height(&mut self, id: AnnotationId, revision: u64, height: f32) -> bool {
        let Some(index) = self
            .annotations
            .entries
            .iter()
            .position(|e| e.annotation.id == id && e.annotation.revision == revision)
        else {
            return false;
        };
        let entry = &mut self.annotations.entries[index];
        if entry.measured == Some((revision, height)) {
            return false;
        }
        entry.measured = Some((revision, height));
        let row = self.annotation_rows.get(index).copied();
        if let Some(row) = row.filter(|&row| row != super::state::NONE)
            && self.list.set_height(RowKey(u64::from(row)), height).is_ok()
        {
            self.revision += 1;
        }
        true
    }

    pub(crate) fn annotation_height(&self, index: u32, m: &Metrics) -> f32 {
        let entry = self.annotations.entry(index);
        match entry.measured {
            Some((revision, height)) if revision == entry.annotation.revision => height,
            _ => (m.line_h * ESTIMATED_LINES).round(),
        }
    }

    /// The slot an annotation row paints, its file's unit, and its title.
    pub(crate) fn annotation_slot(&self, index: u32) -> (AnnotationSlot, u32, Arc<str>) {
        let entry = self.annotations.entry(index);
        let a = &entry.annotation;
        let unit = self.unit_of(a.anchor.file).unwrap_or(0);
        let slot = AnnotationSlot {
            id: a.id,
            side: a.anchor.side,
            lines: a.anchor.lines.clone(),
            outdated: entry.outdated,
            revision: a.revision,
        };
        let side = match a.anchor.side {
            Side::Old => "old",
            Side::New => "new",
        };
        let title = if entry.outdated {
            format!(
                "Outdated annotation on {side} line {}",
                a.anchor.lines.start + 1
            )
        } else {
            format!("Annotation on {side} line {}", a.anchor.lines.start + 1)
        };
        (slot, unit, title.into())
    }

    /// Where every annotation goes. See [`Placement`].
    pub(crate) fn annotation_placement(&self) -> Placement {
        let mut placement = Placement::default();
        for (i, entry) in self.annotations.entries.iter().enumerate() {
            let anchor = &entry.annotation.anchor;
            let Some(unit) = self.unit_of(anchor.file) else {
                continue;
            };
            let Some((seg, file)) = self.locate(unit) else {
                continue;
            };
            let last = anchor.lines.end.max(anchor.lines.start + 1) - 1;
            let index = store_index(&self.segments[seg].doc, file, anchor.side, last);
            match index {
                Some(index) if !entry.outdated => placement
                    .at_line
                    .entry((unit, anchor.side, index))
                    .or_default()
                    .push(i as u32),
                _ => placement.outdated.entry(unit).or_default().push(i as u32),
            }
        }
        placement
    }

    /// Annotations anchored in `unit`'s file.
    pub(crate) fn file_annotations(&self, unit: u32) -> u32 {
        if self.annotations.entries.is_empty() {
            return 0;
        }
        let file = self.file_id(unit);
        self.annotations
            .entries
            .iter()
            .filter(|e| e.annotation.anchor.file == file)
            .count() as u32
    }

    /// Current annotations whose anchor's last line a gap hides.
    pub(crate) fn gap_annotations(&self, segment: &Segment, gap: diffy_diff::GapRow) -> u32 {
        if self.annotations.entries.is_empty() {
            return 0;
        }
        let unit = segment.unit(gap.id.file);
        let file = self.file_id(unit);
        self.annotations
            .entries
            .iter()
            .filter(|e| !e.outdated && e.annotation.anchor.file == file)
            .filter(|e| {
                let a = &e.annotation.anchor;
                let last = a.lines.end.max(a.lines.start + 1) - 1;
                let start = match a.side {
                    Side::Old => gap.old_start,
                    Side::New => gap.new_start,
                };
                // Gaps exist only in whole files, where store index and
                // source line agree.
                (start..start + gap.hidden).contains(&last)
            })
            .count() as u32
    }

    /// An anchor on `lines` of `file`'s current revision, when they exist.
    pub(crate) fn anchor_for(
        &self,
        file: FileId,
        side: Side,
        lines: Range<u32>,
    ) -> Option<DiffAnchor> {
        let (seg, _) = self.locate(self.unit_of(file)?)?;
        let anchor = DiffAnchor {
            file,
            revision: Revision(self.segments[seg].revision),
            side,
            lines,
        };
        self.anchor_is_current(&anchor).then_some(anchor)
    }

    pub fn selected_lines(&self) -> Option<DiffAnchor> {
        let s = self.selection?;
        let (side_a, unit_a, index_a) = decode_key(s.anchor.block);
        let (side_b, unit_b, index_b) = decode_key(s.focus.block);
        if side_a != side_b || unit_a != unit_b {
            return None;
        }
        let (seg, file) = self.locate(unit_a)?;
        let doc = &self.segments[seg].doc;
        let a = source_line(doc, file, side_a, index_a);
        let b = source_line(doc, file, side_a, index_b);
        self.anchor_for(self.file_id(unit_a), side_a, a.min(b)..a.max(b) + 1)
    }

    /// An anchor on the selected lines when the selection stays on one
    /// side of one file, else on the focused row's line.
    pub(crate) fn annotate_selection_or_focus(&self) -> Option<DiffAnchor> {
        if let Some(s) = self.selection {
            let (side_a, unit_a, index_a) = decode_key(s.anchor.block);
            let (side_b, unit_b, index_b) = decode_key(s.focus.block);
            if side_a == side_b && unit_a == unit_b {
                let (seg, file) = self.locate(unit_a)?;
                let doc = &self.segments[seg].doc;
                let a = source_line(doc, file, side_a, index_a);
                let b = source_line(doc, file, side_a, index_b);
                return self.anchor_for(self.file_id(unit_a), side_a, a.min(b)..a.max(b) + 1);
            }
        }
        match self.focused_target()? {
            DiffTarget::Source(point) => {
                self.anchor_for(point.file, point.side, point.line..point.line + 1)
            }
            _ => None,
        }
    }
}
