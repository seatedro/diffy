//! Semantic rows to a [`DiffDocument`]: hunks, aligned rows and spans.

use std::collections::HashSet;
use std::ops::Range;

use diffy_diff::{
    AlignedRow, ChangeIntensity, DiffDocument, DocumentBuilder, FileBuilder, FileMeta,
    LineGroupSpan, SemanticSpan, Side, SourceCoverage, TextStore,
};
use diffy_semantic::{Move, OutlineItem, Rename, Row};

use crate::line::line_changes;
use crate::{Error, Result};

/// What a semantic diff knows beyond its rows.
#[derive(Default)]
pub struct Semantics {
    pub outline: Vec<OutlineItem>,
    pub renames: Vec<Rename>,
    pub moves: Vec<Move>,
    pub groups: Vec<LineGroupSpan>,
}

/// A file shown whole on one `side`: added or deleted.
pub(crate) fn whole_file(
    meta: FileMeta,
    old: TextStore,
    new: TextStore,
    side: Side,
) -> Result<DiffDocument> {
    let lines = match side {
        Side::Old => old.line_count(),
        Side::New => new.line_count(),
    };
    let (old, new) = match side {
        Side::Old => (old, TextStore::default()),
        Side::New => (TextStore::default(), new),
    };
    build(meta, old, new, |file| {
        if lines > 0 {
            match side {
                Side::Old => file.hunk(0, 0).change(lines, 0),
                Side::New => file.hunk(0, 0).change(0, lines),
            };
        }
    })
}

pub(crate) fn build(
    meta: FileMeta,
    old: TextStore,
    new: TextStore,
    shape: impl FnOnce(&mut FileBuilder<'_>),
) -> Result<DiffDocument> {
    let mut builder = DocumentBuilder::new();
    let mut file = builder.file(meta, SourceCoverage::Full, old, new);
    shape(&mut file);
    file.finish()
        .map_err(|error| Error::Build(error.to_string()))?;
    Ok(builder.build())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Context,
    Change,
}

#[derive(Clone, Copy)]
struct Segment {
    kind: Kind,
    old: u32,
    new: u32,
    shown: bool,
}

struct HunkRange {
    old: Range<u32>,
    new: Range<u32>,
    segments: Range<usize>,
}

/// Drops rows' lines past the end of a side and spans that are empty,
/// out of bounds, split a character, or overlap an earlier span.
pub(crate) fn clean_rows(chunks: Vec<Vec<Row>>, old: &TextStore, new: &TextStore) -> Vec<Vec<Row>> {
    let side = |store: &TextStore, line: Option<u32>, spans: Vec<SemanticSpan>| {
        let line = line.filter(|&l| l < store.line_count());
        let spans = line
            .and_then(|l| store.display_line(l))
            .map(|text| clean_spans(spans, text))
            .unwrap_or_default();
        (line, spans)
    };
    chunks
        .into_iter()
        .map(|chunk| {
            chunk
                .into_iter()
                .map(|row| {
                    let [old_spans, new_spans] = row.spans;
                    let (old_line, old_spans) = side(old, row.old, old_spans);
                    let (new_line, new_spans) = side(new, row.new, new_spans);
                    Row {
                        old: old_line,
                        new: new_line,
                        spans: [old_spans, new_spans],
                    }
                })
                .collect()
        })
        .collect()
}

/// `spans` clipped to `text`, sorted, without empty, overlapping, or
/// mid-character spans.
pub fn clean_spans(spans: Vec<SemanticSpan>, text: &str) -> Vec<SemanticSpan> {
    let mut out: Vec<SemanticSpan> = spans
        .into_iter()
        .filter_map(|span| {
            let start = span.start as usize;
            let end = (span.end as usize).min(text.len());
            (start < end && text.is_char_boundary(start) && text.is_char_boundary(end)).then_some(
                SemanticSpan {
                    start: start as u32,
                    end: end as u32,
                    intensity: span.intensity,
                },
            )
        })
        .collect();
    out.sort_by_key(|span| (span.start, span.end));
    let mut end = 0;
    out.retain(|span| {
        let keep = span.start >= end;
        if keep {
            end = span.end;
        }
        keep
    });
    out
}

/// One file from cleaned (see [`clean_spans`]) semantic rows; differing lines
/// between rows are line-diffed.
pub fn document_from_rows(
    meta: FileMeta,
    old: TextStore,
    new: TextStore,
    chunks: Vec<Vec<Row>>,
    structural: bool,
    semantics: Semantics,
) -> Result<DiffDocument> {
    let segments = canonical_segments(&chunks, &old, &new);
    let hunks = hunk_ranges(&segments);
    let mut rows: Vec<Vec<AlignedRow>> = hunks.iter().map(|_| Vec::new()).collect();
    let mut spans: [Vec<(u32, Vec<SemanticSpan>)>; 2] = [Vec::new(), Vec::new()];
    let mut seen = [HashSet::new(), HashSet::new()];
    for row in chunks.iter().flatten() {
        let old_hunk = row.old.and_then(|line| find_hunk(&hunks, Side::Old, line));
        let new_hunk = row.new.and_then(|line| find_hunk(&hunks, Side::New, line));
        if let Some(h) = old_hunk.or(new_hunk) {
            let range = &hunks[h];
            let aligned = AlignedRow {
                old: row.old.filter(|line| range.old.contains(line)),
                new: row.new.filter(|line| range.new.contains(line)),
            };
            if aligned.old.is_some() || aligned.new.is_some() {
                rows[h].push(aligned);
            }
        }
        for (side, line, hunk) in [
            (Side::Old, row.old, old_hunk),
            (Side::New, row.new, new_hunk),
        ] {
            let s = side as usize;
            if let (Some(line), Some(_)) = (line, hunk)
                && !row.spans[s].is_empty()
                && seen[s].insert(line)
            {
                spans[s].push((line, row.spans[s].clone()));
            }
        }
    }
    build(meta, old, new, |file| {
        file.structural(structural);
        file.outline(semantics.outline);
        file.renames(semantics.renames);
        file.moves(semantics.moves);
        file.groups(semantics.groups);
        for (hunk, hunk_rows) in hunks.iter().zip(rows) {
            file.hunk(hunk.old.start, hunk.new.start);
            for segment in &segments[hunk.segments.clone()] {
                match segment.kind {
                    Kind::Context => file.context(segment.old),
                    Kind::Change => file.change(segment.old, segment.new),
                };
            }
            for row in hunk_rows {
                file.row(row);
            }
        }
        for side in [Side::Old, Side::New] {
            for (line, line_spans) in &spans[side as usize] {
                file.spans(side, *line, line_spans);
            }
        }
    })
}

fn is_context_row(row: &Row, old: &TextStore, new: &TextStore, lo: u32, ln: u32) -> bool {
    old.line(lo) == new.line(ln)
        && row
            .spans
            .iter()
            .flatten()
            .all(|span| span.intensity == ChangeIntensity::UnchangedContext)
}

fn canonical_segments(chunks: &[Vec<Row>], old: &TextStore, new: &TextStore) -> Vec<Segment> {
    let mut segments = Vec::new();
    let (mut o, mut n) = (0u32, 0u32);
    for chunk in chunks {
        let mut first = true;
        for row in chunk {
            let lo = row.old.filter(|&line| line >= o);
            let ln = row.new.filter(|&line| line >= n);
            if lo.is_none() && ln.is_none() {
                continue;
            }
            let (to, tn) = (lo.unwrap_or(o), ln.unwrap_or(n));
            push_unmentioned(&mut segments, old, new, o..to, n..tn, !first);
            first = false;
            let kind = match (lo, ln) {
                (Some(a), Some(b)) if is_context_row(row, old, new, a, b) => Kind::Context,
                _ => Kind::Change,
            };
            segments.push(Segment {
                kind,
                old: u32::from(lo.is_some()),
                new: u32::from(ln.is_some()),
                shown: true,
            });
            o = to + u32::from(lo.is_some());
            n = tn + u32::from(ln.is_some());
        }
    }
    push_unmentioned(
        &mut segments,
        old,
        new,
        o..old.line_count(),
        n..new.line_count(),
        false,
    );
    segments
}

fn push_unmentioned(
    segments: &mut Vec<Segment>,
    old: &TextStore,
    new: &TextStore,
    old_lines: Range<u32>,
    new_lines: Range<u32>,
    inside: bool,
) {
    let (old_len, new_len) = (old_lines.len() as u32, new_lines.len() as u32);
    if old_len == 0 && new_len == 0 {
        return;
    }
    let equal = old_len == new_len
        && old_lines
            .clone()
            .zip(new_lines.clone())
            .all(|(a, b)| old.line(a) == new.line(b));
    if equal {
        segments.push(Segment {
            kind: Kind::Context,
            old: old_len,
            new: new_len,
            shown: inside,
        });
        return;
    }
    let changes = line_changes(
        lines_bytes(old, old_lines.clone()),
        lines_bytes(new, new_lines.clone()),
    );
    let mut pieces = Vec::with_capacity(changes.len() * 2 + 1);
    let (mut o, mut n) = (0u32, 0u32);
    for change in &changes {
        if change.old.start > o {
            pieces.push((Kind::Context, change.old.start - o, change.new.start - n));
        }
        pieces.push((
            Kind::Change,
            change.old.len() as u32,
            change.new.len() as u32,
        ));
        o = change.old.end;
        n = change.new.end;
    }
    if o < old_len {
        pieces.push((Kind::Context, old_len - o, new_len - n));
    }
    let last = pieces.len().saturating_sub(1);
    for (index, (kind, old, new)) in pieces.into_iter().enumerate() {
        let edge = index == 0 || index == last;
        segments.push(Segment {
            kind,
            old,
            new,
            shown: kind == Kind::Change || !edge || inside,
        });
    }
}

fn lines_bytes(store: &TextStore, lines: Range<u32>) -> &[u8] {
    let text = store.as_str();
    let offset = |line: u32| {
        store
            .line_range(line)
            .map_or(text.len(), |range| range.start)
    };
    &text.as_bytes()[offset(lines.start)..offset(lines.end)]
}

fn hunk_ranges(segments: &[Segment]) -> Vec<HunkRange> {
    let mut hunks = Vec::new();
    let (mut o, mut n) = (0u32, 0u32);
    let mut open: Option<(u32, u32, usize, bool)> = None;
    for (index, segment) in segments.iter().enumerate() {
        let in_hunk = segment.shown || segment.kind == Kind::Change;
        match (&mut open, in_hunk) {
            (None, true) => open = Some((o, n, index, segment.kind == Kind::Change)),
            (Some(state), true) => state.3 |= segment.kind == Kind::Change,
            (Some(_), false) => close(&mut hunks, open.take(), o, n, index),
            (None, false) => {}
        }
        o += segment.old;
        n += segment.new;
    }
    close(&mut hunks, open, o, n, segments.len());
    hunks
}

fn close(
    hunks: &mut Vec<HunkRange>,
    open: Option<(u32, u32, usize, bool)>,
    o: u32,
    n: u32,
    end: usize,
) {
    if let Some((start_o, start_n, start, true)) = open {
        hunks.push(HunkRange {
            old: start_o..o,
            new: start_n..n,
            segments: start..end,
        });
    }
}

fn find_hunk(hunks: &[HunkRange], side: Side, line: u32) -> Option<usize> {
    let range = |h: &HunkRange| match side {
        Side::Old => h.old.clone(),
        Side::New => h.new.clone(),
    };
    let index = hunks.partition_point(|h| range(h).end <= line);
    (index < hunks.len() && range(&hunks[index]).contains(&line)).then_some(index)
}
