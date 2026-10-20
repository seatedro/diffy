//! Source coverage and hydration: replacing a parsed patch's hunk
//! fragments with the exact old and new files they came from, after
//! checking that the files really are the patch's sides.

use std::fmt;

use crate::model::{BlockKind, DiffDocument, FileStatus, Side, lines_before};
use crate::projection::GapId;
use crate::text::TextStore;

/// The whole old and new text of one file. `None` is a side that does not
/// exist (the old side of an added file, the new side of a deleted one),
/// never a side that has not been loaded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileSources {
    pub old: Option<TextStore>,
    pub new: Option<TextStore>,
}

/// How much of a file's text a document holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceCoverage {
    /// Both whole files: syntax sees real lexical context and every gap
    /// can be expanded.
    Full,
    /// Only the lines the patch shows.
    PatchOnly,
}

/// Lines a collapsed gap holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContextLen {
    /// Known from the texts, or from hunk headers for a patch: this many
    /// lines exist, though a patch-only document cannot show them.
    Lines(u32),
    /// A patch does not say how far a file runs past its last hunk.
    Unknown,
}

/// Why [`DiffDocument::hydrate_file`] refused a pair of sources. Lines are
/// zero-based source lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HydrationError {
    NoSuchFile {
        file: u32,
    },
    /// Binary content has no text lines to check against.
    Binary,
    /// The file has this side but no source was supplied for it.
    MissingSide(Side),
    /// A source was supplied for a side the file does not have.
    UnexpectedSide(Side),
    /// The source's line differs from the patch's line at this position.
    Mismatch {
        side: Side,
        line: u32,
    },
    /// A hunk reaches past the end of the source.
    OutOfRange {
        side: Side,
        line: u32,
    },
    /// Unchanged lines between or after hunks differ between the sides,
    /// or their counts do.
    GapMismatch {
        old_line: u32,
        new_line: u32,
    },
    /// The source's final newline disagrees with the patch.
    Eof(Side),
}

impl fmt::Display for HydrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchFile { file } => write!(f, "no file {file} in the diff"),
            Self::Binary => f.write_str("binary files have no text sources"),
            Self::MissingSide(side) => write!(f, "no {side:?} source supplied"),
            Self::UnexpectedSide(side) => write!(f, "the file has no {side:?} side"),
            Self::Mismatch { side, line } => {
                write!(
                    f,
                    "{side:?} source line {} does not match the patch",
                    line + 1
                )
            }
            Self::OutOfRange { side, line } => {
                write!(f, "{side:?} source ends before line {}", line + 1)
            }
            Self::GapMismatch { old_line, new_line } => write!(
                f,
                "unchanged old line {} and new line {} differ",
                old_line + 1,
                new_line + 1
            ),
            Self::Eof(side) => write!(f, "{side:?} source's final newline disagrees"),
        }
    }
}

impl std::error::Error for HydrationError {}

impl DiffDocument {
    pub fn coverage(&self, file: u32) -> SourceCoverage {
        if self.files().partial[file as usize] {
            SourceCoverage::PatchOnly
        } else {
            SourceCoverage::Full
        }
    }

    /// Lines `gap` holds, including those a patch-only file cannot show.
    pub fn context_len(&self, gap: GapId) -> ContextLen {
        let f = gap.file as usize;
        if !self.files().partial[f] {
            return ContextLen::Lines(match gap.hunk {
                Some(h) => self.gap_above(h),
                None => self.gap_after(gap.file),
            });
        }
        let Some(hunk) = gap.hunk else {
            return ContextLen::Unknown;
        };
        let h = self.hunks();
        let hi = hunk as usize;
        let start = lines_before(h.new_start[hi], h.new_len[hi]);
        let prev_end = (self.files().hunks[f].start < hunk).then(|| {
            let p = hi - 1;
            lines_before(h.new_start[p], h.new_len[p]) + h.new_len[p]
        });
        ContextLen::Lines(start - prev_end.unwrap_or(0))
    }

    /// This document with file `file`'s patch fragments replaced by its
    /// whole sources, so its gaps expand and syntax sees each side whole.
    /// Hunks, counts, paths, modes, and status stay as they are.
    ///
    /// Every hunk line must match the source at its declared position, the
    /// unchanged lines between and after hunks must be equal on both sides,
    /// and final newlines must agree; anything else means the sources are
    /// not the patch's, and is refused. O(file bytes).
    pub fn hydrate_file(
        &self,
        file: u32,
        sources: FileSources,
    ) -> Result<DiffDocument, HydrationError> {
        if file >= self.file_count() {
            return Err(HydrationError::NoSuchFile { file });
        }
        let meta = &self.files().meta[file as usize];
        if meta.binary {
            return Err(HydrationError::Binary);
        }
        let side = |source: Option<TextStore>, exists: bool, side: Side| match (source, exists) {
            (Some(text), true) => Ok(text),
            (None, false) => Ok(TextStore::default()),
            (None, true) => Err(HydrationError::MissingSide(side)),
            (Some(_), false) => Err(HydrationError::UnexpectedSide(side)),
        };
        let old = side(sources.old, meta.status != FileStatus::Added, Side::Old)?;
        let new = side(sources.new, meta.status != FileStatus::Deleted, Side::New)?;
        self.check_sources(file, &old, &new)?;
        let mut out = DiffDocument::default();
        for f in 0..self.file_count() {
            let full = (f == file).then(|| (old.clone(), new.clone()));
            out.push_file_from(self, f, full);
        }
        debug_assert_eq!(out.verify_integrity(), Ok(()));
        Ok(out)
    }

    fn check_sources(
        &self,
        file: u32,
        old: &TextStore,
        new: &TextStore,
    ) -> Result<(), HydrationError> {
        let f = file as usize;
        let (h, b) = (self.hunks(), self.blocks());
        let stores = [self.text(file, Side::Old), self.text(file, Side::New)];
        let sources = [old, new];
        // Next line after the previous hunk, per side.
        let mut ends = [0u32; 2];
        for hunk in self.files().hunks[f].clone() {
            let hi = hunk as usize;
            let before = [
                lines_before(h.old_start[hi], h.old_len[hi]),
                lines_before(h.new_start[hi], h.new_len[hi]),
            ];
            check_gap(sources, ends, before[0] - ends[0], before[1] - ends[1])?;
            for block in h.blocks[hi].clone() {
                let bi = block as usize;
                let lines = [
                    (
                        Side::Old,
                        b.old_store[bi],
                        b.old_line[bi] - 1,
                        b.old_len[bi],
                    ),
                    (
                        Side::New,
                        b.new_store[bi],
                        b.new_line[bi] - 1,
                        b.new_len[bi],
                    ),
                ];
                for (side, store_at, line_at, len) in lines {
                    let s = side as usize;
                    for k in 0..len {
                        let line = line_at + k;
                        let actual = sources[s]
                            .line(line)
                            .ok_or(HydrationError::OutOfRange { side, line })?;
                        if stores[s].line(store_at + k) != Some(actual) {
                            return Err(HydrationError::Mismatch { side, line });
                        }
                    }
                }
                debug_assert!(b.kind[bi] != BlockKind::Context || b.old_len[bi] == b.new_len[bi]);
            }
            ends = [before[0] + h.old_len[hi], before[1] + h.new_len[hi]];
        }
        let tails = [
            old.line_count().saturating_sub(ends[0]),
            new.line_count().saturating_sub(ends[1]),
        ];
        check_gap(sources, ends, tails[0], tails[1])?;
        for side in [Side::Old, Side::New] {
            let s = side as usize;
            let store = stores[s];
            let reaches_end = ends[s] == sources[s].line_count();
            // A patch marks a missing final newline only on a side's last
            // line; with a tail, both sides share that line's ending.
            let expected = if reaches_end && store.line_count() > 0 {
                store.no_newline_at_eof()
            } else if reaches_end {
                // No hunk lines on this side, so its last line (if any) is
                // unchanged and followed by the other side's hunk lines:
                // it ends in a newline.
                false
            } else {
                if store.no_newline_at_eof() {
                    return Err(HydrationError::Eof(side));
                }
                sources[1 - s].no_newline_at_eof()
            };
            if sources[s].no_newline_at_eof() != expected {
                return Err(HydrationError::Eof(side));
            }
        }
        Ok(())
    }
}

/// Checks a run of unchanged lines starting at `starts`: `old_len` and
/// `new_len` lines that must be equally many and equal line for line.
fn check_gap(
    [old, new]: [&TextStore; 2],
    [old_at, new_at]: [u32; 2],
    old_len: u32,
    new_len: u32,
) -> Result<(), HydrationError> {
    let mismatch = |k: u32| HydrationError::GapMismatch {
        old_line: old_at + k,
        new_line: new_at + k,
    };
    if old_len != new_len {
        return Err(mismatch(old_len.min(new_len)));
    }
    for k in 0..old_len {
        let (o, n) = (old.line(old_at + k), new.line(new_at + k));
        if o.is_none() || o != n {
            return Err(mismatch(k));
        }
    }
    Ok(())
}
