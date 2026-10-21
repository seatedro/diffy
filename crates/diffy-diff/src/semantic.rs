use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use crate::model::{DiffDocument, Side, lines_before};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeIntensity {
    Novel,
    NovelWord,
    UnchangedContext,
    Renamed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SemanticSpan {
    pub start: u32,
    pub end: u32,
    pub intensity: ChangeIntensity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AlignedRow {
    pub old: Option<u32>,
    pub new: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticError {
    HunkRows,
    Row { hunk: u32, row: u32 },
    SpanLine { side: Side, line: u32 },
    Span { side: Side, line: u32 },
}

impl fmt::Display for SemanticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HunkRows => f.write_str("aligned rows do not match the hunks"),
            Self::Row { hunk, row } => write!(f, "row {row} of hunk {hunk} is outside the hunk"),
            Self::SpanLine { side, line } => {
                write!(f, "{side:?} line {} has no changed text", line + 1)
            }
            Self::Span { side, line } => {
                write!(f, "a span of {side:?} line {} is out of bounds", line + 1)
            }
        }
    }
}

impl std::error::Error for SemanticError {}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LineSpans {
    lines: Vec<u32>,
    ranges: Vec<Range<u32>>,
    spans: Vec<SemanticSpan>,
}

impl LineSpans {
    pub(crate) fn push(&mut self, line: u32, spans: &[SemanticSpan]) {
        let start = self.spans.len() as u32;
        self.spans.extend_from_slice(spans);
        self.lines.push(line);
        self.ranges.push(start..self.spans.len() as u32);
    }

    fn get(&self, line: u32) -> &[SemanticSpan] {
        match self.lines.binary_search(&line) {
            Ok(i) => {
                let r = &self.ranges[i];
                &self.spans[r.start as usize..r.end as usize]
            }
            Err(_) => &[],
        }
    }

    fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemChange {
    Added,
    Removed,
    Renamed,
    Moved,
    Edited(u32),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Move {
    pub old_line: u32,
    pub old_lines: u32,
    pub new_line: u32,
    pub lines: u32,
    pub from: Option<Arc<str>>,
    pub into: Option<Arc<str>>,
    pub item: Option<Arc<str>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LineGroupSpan {
    pub old: Range<u32>,
    pub new: Range<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Rename {
    pub old: Arc<str>,
    pub new: Arc<str>,
    pub uses: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OutlineItem {
    pub side: Side,
    pub line: u32,
    pub kind: Arc<str>,
    pub name: Arc<str>,
    pub change: ItemChange,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileSemantics {
    pub(crate) structural: bool,
    pub(crate) outline: Vec<OutlineItem>,
    pub(crate) renames: Vec<Rename>,
    pub(crate) moves: Vec<Move>,
    pub(crate) groups: Vec<LineGroupSpan>,
    pub(crate) rows: Vec<AlignedRow>,
    pub(crate) hunk_rows: Vec<Range<u32>>,
    pub(crate) spans: [LineSpans; 2],
}

impl FileSemantics {
    pub fn structural(&self) -> bool {
        self.structural
    }

    pub fn outline(&self) -> &[OutlineItem] {
        &self.outline
    }

    pub fn renames(&self) -> &[Rename] {
        &self.renames
    }

    pub fn moves(&self) -> &[Move] {
        &self.moves
    }

    pub fn groups(&self) -> &[LineGroupSpan] {
        &self.groups
    }

    pub fn rows(&self, local_hunk: u32) -> &[AlignedRow] {
        self.hunk_rows
            .get(local_hunk as usize)
            .map_or(&[], |r| &self.rows[r.start as usize..r.end as usize])
    }

    pub fn all_rows(&self) -> &[AlignedRow] {
        &self.rows
    }

    pub fn spans(&self, side: Side, line: u32) -> &[SemanticSpan] {
        self.spans[side as usize].get(line)
    }

    pub fn span_lines(&self, side: Side) -> &[u32] {
        &self.spans[side as usize].lines
    }

    pub(crate) fn is_empty(&self) -> bool {
        !self.structural && self.rows.is_empty() && self.spans.iter().all(LineSpans::is_empty)
    }

    pub(crate) fn verify(&self, doc: &DiffDocument, file: u32) -> Result<(), SemanticError> {
        let hunks = doc.files().hunks[file as usize].clone();
        if self.hunk_rows.len() != hunks.len() {
            return Err(SemanticError::HunkRows);
        }
        let mut next = 0;
        for (local, range) in self.hunk_rows.iter().enumerate() {
            if range.start != next || range.end < range.start {
                return Err(SemanticError::HunkRows);
            }
            next = range.end;
            let hunk = hunks.start + local as u32;
            let (old, new) = (
                doc.hunk_lines(hunk, Side::Old),
                doc.hunk_lines(hunk, Side::New),
            );
            for (k, row) in self.rows[range.start as usize..range.end as usize]
                .iter()
                .enumerate()
            {
                let inside = |line: Option<u32>, lines: &Range<u32>| {
                    line.is_none_or(|line| lines.contains(&line))
                };
                if (row.old.is_none() && row.new.is_none())
                    || !inside(row.old, &old)
                    || !inside(row.new, &new)
                {
                    return Err(SemanticError::Row {
                        hunk: local as u32,
                        row: k as u32,
                    });
                }
            }
        }
        if next as usize != self.rows.len() {
            return Err(SemanticError::HunkRows);
        }
        for side in [Side::Old, Side::New] {
            let spans = &self.spans[side as usize];
            let mut previous = None;
            for (i, &line) in spans.lines.iter().enumerate() {
                if previous.is_some_and(|p| p >= line) {
                    return Err(SemanticError::SpanLine { side, line });
                }
                previous = Some(line);
                let text = doc
                    .hunk_at(file, side, line)
                    .and_then(|_| doc.store_line(file, side, line))
                    .and_then(|store| doc.text(file, side).display_line(store))
                    .ok_or(SemanticError::SpanLine { side, line })?;
                let r = &spans.ranges[i];
                let mut end = 0;
                for span in &spans.spans[r.start as usize..r.end as usize] {
                    let (s, e) = (span.start as usize, span.end as usize);
                    if s >= e
                        || s < end
                        || e > text.len()
                        || !text.is_char_boundary(s)
                        || !text.is_char_boundary(e)
                    {
                        return Err(SemanticError::Span { side, line });
                    }
                    end = e;
                }
            }
        }
        Ok(())
    }
}

impl DiffDocument {
    pub fn hunk_lines(&self, hunk: u32, side: Side) -> Range<u32> {
        let h = self.hunks();
        let hi = hunk as usize;
        let (start, len) = match side {
            Side::Old => (h.old_start[hi], h.old_len[hi]),
            Side::New => (h.new_start[hi], h.new_len[hi]),
        };
        let before = lines_before(start, len);
        before..before + len
    }

    pub fn hunk_at(&self, file: u32, side: Side, line: u32) -> Option<u32> {
        let range = self.files().hunks.get(file as usize)?.clone();
        let (mut lo, mut hi) = (range.start, range.end);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.hunk_lines(mid, side).end <= line {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        (lo < range.end && self.hunk_lines(lo, side).contains(&line)).then_some(lo)
    }

    pub fn store_line(&self, file: u32, side: Side, line: u32) -> Option<u32> {
        if file >= self.file_count() {
            return None;
        }
        let store = self.text(file, side);
        if !self.files().partial[file as usize] {
            return (line < store.line_count()).then_some(line);
        }
        let hunk = self.hunk_at(file, side, line)?;
        let b = self.blocks();
        for block in self.hunks().blocks[hunk as usize].clone() {
            let bi = block as usize;
            let (first, len, at) = match side {
                Side::Old => (b.old_line[bi] - 1, b.old_len[bi], b.old_store[bi]),
                Side::New => (b.new_line[bi] - 1, b.new_len[bi], b.new_store[bi]),
            };
            if (first..first + len).contains(&line) {
                return Some(at + line - first);
            }
        }
        None
    }

    pub fn semantic_spans(&self, file: u32, side: Side, line: u32) -> &[SemanticSpan] {
        self.semantics(file)
            .map_or(&[], |semantics| semantics.spans(side, line))
    }

    pub fn aligned_rows(&self, hunk: u32) -> &[AlignedRow] {
        let Some(&file) = self.hunks().file.get(hunk as usize) else {
            return &[];
        };
        let local = hunk - self.files().hunks[file as usize].start;
        self.semantics(file)
            .map_or(&[], |semantics| semantics.rows(local))
    }

    pub fn prefers_structural(&self, file: u32) -> bool {
        self.semantics(file).is_some_and(FileSemantics::structural)
    }
}
