//! Source line correspondence between two revisions of a file, for moving
//! selections, expanded context, and scroll anchors onto a new revision.
//!
//! A line corresponds only where a line diff of the two revisions' whole
//! sources found it unchanged; equal paths, row numbers, or byte offsets
//! prove nothing.

use std::fmt;
use std::ops::Range;

use crate::compute::line_changes;
use crate::model::Side;
use crate::session::{FileDiffSnapshot, FileId, Revision};
use crate::source::SourceCoverage;
use crate::text::TextStore;

/// Where a source line of the earlier revision went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineMap {
    /// Unchanged, now at this line.
    Kept(u32),
    /// Changed or deleted. `at` is the later revision's line where the
    /// replacement starts: the nearest surviving boundary, equal to the
    /// line count when the replacement runs to the end.
    Replaced { at: u32 },
}

/// One side's unchanged runs: `(from_start, to_start, len)`, sorted and
/// disjoint on both revisions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SideMap {
    from_lines: u32,
    to_lines: u32,
    runs: Vec<(u32, u32, u32)>,
}

impl SideMap {
    fn new(from: &TextStore, to: &TextStore) -> Self {
        let mut runs = Vec::new();
        let (mut old_at, mut new_at) = (0, 0);
        for change in line_changes(from, to) {
            if change.old.start > old_at {
                runs.push((old_at, new_at, change.old.start - old_at));
            }
            old_at = change.old.end;
            new_at = change.new.end;
        }
        if from.line_count() > old_at {
            runs.push((old_at, new_at, from.line_count() - old_at));
        }
        Self {
            from_lines: from.line_count(),
            to_lines: to.line_count(),
            runs,
        }
    }

    fn map(&self, line: u32) -> LineMap {
        let at = self.runs.partition_point(|r| r.0 <= line);
        let Some(&(from, to, len)) = at.checked_sub(1).map(|i| &self.runs[i]) else {
            return LineMap::Replaced { at: 0 };
        };
        if line < from + len {
            LineMap::Kept(to + line - from)
        } else {
            LineMap::Replaced {
                at: (to + len).min(self.to_lines),
            }
        }
    }

    fn verify(&self) -> bool {
        let mut ends = (0, 0);
        self.runs.iter().all(|&(from, to, len)| {
            let ok = len > 0
                && from >= ends.0
                && to >= ends.1
                && from + len <= self.from_lines
                && to + len <= self.to_lines;
            ends = (from + len, to + len);
            ok
        })
    }
}

/// Line correspondence for both sides of one file between two revisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRemap {
    pub file: FileId,
    pub from: Revision,
    pub to: Revision,
    sides: [SideMap; 2],
}

/// Why [`SourceRemap::between`] could not map two snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemapError {
    DifferentFiles,
    /// `to` is not a later revision than `from`.
    NotNewer,
    /// A snapshot lacks whole sources, so unchanged lines cannot be told
    /// from lines the patch left out.
    PatchOnly,
}

impl fmt::Display for RemapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "source remap: {self:?}")
    }
}

impl std::error::Error for RemapError {}

impl SourceRemap {
    /// Maps each side of `from` to the same side of `to` by a line diff of
    /// their whole sources. O(lines) plus the diff; run it off the UI
    /// thread for large files.
    pub fn between(from: &FileDiffSnapshot, to: &FileDiffSnapshot) -> Result<Self, RemapError> {
        if from.id != to.id {
            return Err(RemapError::DifferentFiles);
        }
        if to.revision <= from.revision {
            return Err(RemapError::NotNewer);
        }
        if from.coverage() != SourceCoverage::Full || to.coverage() != SourceCoverage::Full {
            return Err(RemapError::PatchOnly);
        }
        let side = |side| SideMap::new(from.diff.text(0, side), to.diff.text(0, side));
        let remap = Self {
            file: from.id,
            from: from.revision,
            to: to.revision,
            sides: [side(Side::Old), side(Side::New)],
        };
        debug_assert!(remap.sides.iter().all(SideMap::verify));
        Ok(remap)
    }

    /// Where line `line` of `side` in the earlier revision is now.
    pub fn map_line(&self, side: Side, line: u32) -> LineMap {
        self.sides[side as usize].map(line)
    }

    /// The later revision's lines for `lines`, when every one of them is
    /// unchanged and they stay contiguous; `None` when any was replaced,
    /// so a selection over them should be cleared.
    pub fn map_range(&self, side: Side, lines: Range<u32>) -> Option<Range<u32>> {
        if lines.is_empty() {
            return match self.map_line(side, lines.start) {
                LineMap::Kept(at) | LineMap::Replaced { at } => Some(at..at),
            };
        }
        let LineMap::Kept(start) = self.map_line(side, lines.start) else {
            return None;
        };
        // Every line, not just the ends: a one-for-one replacement inside
        // the range keeps the ends contiguous.
        lines
            .clone()
            .zip(start..)
            .all(|(line, at)| self.map_line(side, line) == LineMap::Kept(at))
            .then(|| start..start + lines.len() as u32)
    }

    /// Whether this remap leads from `current` to `next`.
    pub(crate) fn fits(&self, current: &FileDiffSnapshot, next: &FileDiffSnapshot) -> bool {
        let counts = |snapshot: &FileDiffSnapshot, side| snapshot.diff.text(0, side).line_count();
        self.file == current.id
            && self.from == current.revision
            && self.to == next.revision
            && next.coverage() == SourceCoverage::Full
            && current.coverage() == SourceCoverage::Full
            && [Side::Old, Side::New].iter().all(|&side| {
                let map = &self.sides[side as usize];
                map.from_lines == counts(current, side) && map.to_lines == counts(next, side)
            })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use proptest::prelude::*;

    use super::{LineMap, RemapError, SourceRemap};
    use crate::session::{FileDiffSnapshot, FileId, Revision};
    use crate::{Side, diff_texts, parse_unified, write_unified};

    fn snapshot(revision: u64, new: &str) -> FileDiffSnapshot {
        let doc = diff_texts(Some("f"), Some("f"), Some("base\n"), Some(new), 3);
        FileDiffSnapshot::new(FileId(1), Revision(revision), Arc::new(doc)).unwrap()
    }

    #[test]
    fn unchanged_lines_follow_their_text_and_replaced_ones_are_flagged() {
        let from = snapshot(1, "a\nb\nc\nd\n");
        let to = snapshot(2, "x\na\nB\nc\nd\n");
        let remap = SourceRemap::between(&from, &to).unwrap();
        let lines: Vec<LineMap> = (0..4).map(|l| remap.map_line(Side::New, l)).collect();
        assert_eq!(
            lines,
            [
                LineMap::Kept(1),
                LineMap::Replaced { at: 2 },
                LineMap::Kept(3),
                LineMap::Kept(4)
            ]
        );
        let ranges = [0..1, 1..3, 2..4, 1..1].map(|r| remap.map_range(Side::New, r));
        assert_eq!(ranges, [Some(1..2), None, Some(3..5), Some(2..2)]);
    }

    // A line replaced one-for-one inside a range leaves its ends kept and
    // contiguous; the range must still be refused.
    #[test]
    fn a_range_with_a_line_replaced_inside_is_not_kept() {
        let from = snapshot(1, "a\nb\nc\n");
        let to = snapshot(2, "a\nB\nc\n");
        let remap = SourceRemap::between(&from, &to).unwrap();
        assert_eq!(remap.map_range(Side::New, 0..3), None);
        assert_eq!(remap.map_range(Side::New, 2..3), Some(2..3));
    }

    #[test]
    fn patch_only_snapshots_cannot_be_remapped() {
        let full = diff_texts(Some("f"), Some("f"), Some("a\n"), Some("b\n"), 3);
        let patch = parse_unified(&write_unified(&full)).unwrap();
        let from = FileDiffSnapshot::new(FileId(1), Revision(1), Arc::new(patch)).unwrap();
        let to = snapshot(2, "c\n");
        assert_eq!(SourceRemap::between(&from, &to), Err(RemapError::PatchOnly));
    }

    fn text() -> impl Strategy<Value = String> {
        let line = prop::sample::select(vec!["a", "b", "c", "}", ""]);
        prop::collection::vec(line, 0..20).prop_map(|lines| lines.join("\n"))
    }

    proptest! {
        #[test]
        fn kept_lines_map_to_equal_lines_in_order(from in text(), to in text()) {
            let (a, b) = (snapshot(1, &from), snapshot(2, &to));
            let remap = SourceRemap::between(&a, &b).unwrap();
            let (old, new) = (a.diff.text(0, Side::New), b.diff.text(0, Side::New));
            let mut last = None;
            for line in 0..old.line_count() {
                if let LineMap::Kept(at) = remap.map_line(Side::New, line) {
                    prop_assert_eq!(old.line(line), new.line(at));
                    prop_assert!(last.is_none_or(|l| l < at));
                    last = Some(at);
                }
            }
        }
    }
}
