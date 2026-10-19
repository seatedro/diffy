use std::fmt;
use std::sync::Arc;

use crate::compute::hunk_groups;
use crate::model::{BlockKind, DiffDocument, FileMeta, FileStatus, IntegrityError, Side};
use crate::myers::Change;
use crate::semantic::{
    AlignedRow, FileSemantics, LineSpans, OutlineItem, SemanticError, SemanticSpan,
};
use crate::source::SourceCoverage;
use crate::text::TextStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    NoHunk,
    EmptyHunk { hunk: u32 },
    HunkOrder { hunk: u32 },
    ChangeOrder { change: u32 },
    OutOfRange { side: Side, line: u32 },
    ContextMismatch { old_line: u32, new_line: u32 },
    GapLength { old_line: u32, new_line: u32 },
    UnusedStore(Side),
    UnexpectedSide(Side),
    DuplicateSpans { side: Side, line: u32 },
    Semantics(SemanticError),
    Integrity(IntegrityError),
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoHunk => f.write_str("lines added before any hunk"),
            Self::EmptyHunk { hunk } => write!(f, "hunk {hunk} has no lines"),
            Self::HunkOrder { hunk } => write!(f, "hunk {hunk} starts before the previous ends"),
            Self::ChangeOrder { change } => write!(f, "change {change} is out of order"),
            Self::OutOfRange { side, line } => {
                write!(f, "{side:?} text ends before line {}", line + 1)
            }
            Self::ContextMismatch { old_line, new_line } => write!(
                f,
                "context old line {} and new line {} differ",
                old_line + 1,
                new_line + 1
            ),
            Self::GapLength { old_line, new_line } => write!(
                f,
                "unchanged lines before old line {} and new line {} differ in number",
                old_line + 1,
                new_line + 1
            ),
            Self::UnusedStore(side) => write!(f, "{side:?} text holds lines no hunk shows"),
            Self::UnexpectedSide(side) => write!(f, "the file has no {side:?} side"),
            Self::DuplicateSpans { side, line } => {
                write!(f, "{side:?} line {} has spans twice", line + 1)
            }
            Self::Semantics(error) => error.fmt(f),
            Self::Integrity(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for BuildError {}

#[derive(Debug, Default)]
pub struct DocumentBuilder {
    doc: DiffDocument,
}

impl DocumentBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn file(
        &mut self,
        meta: FileMeta,
        coverage: SourceCoverage,
        old: TextStore,
        new: TextStore,
    ) -> FileBuilder<'_> {
        FileBuilder {
            target: &mut self.doc,
            meta,
            coverage,
            old,
            new,
            hunks: Vec::new(),
            structural: false,
            outline: Vec::new(),
            spans: [Vec::new(), Vec::new()],
            error: None,
        }
    }

    pub fn file_count(&self) -> u32 {
        self.doc.file_count()
    }

    pub fn build(self) -> DiffDocument {
        self.doc
    }
}

struct HunkDraft {
    old_line: u32,
    new_line: u32,
    section: Arc<str>,
    blocks: Vec<(BlockKind, u32, u32)>,
    rows: Vec<AlignedRow>,
}

pub struct FileBuilder<'a> {
    target: &'a mut DiffDocument,
    meta: FileMeta,
    coverage: SourceCoverage,
    old: TextStore,
    new: TextStore,
    hunks: Vec<HunkDraft>,
    structural: bool,
    outline: Vec<OutlineItem>,
    spans: [Vec<(u32, Vec<SemanticSpan>)>; 2],
    error: Option<BuildError>,
}

impl FileBuilder<'_> {
    pub fn hunk(&mut self, old_line: u32, new_line: u32) -> &mut Self {
        self.hunks.push(HunkDraft {
            old_line,
            new_line,
            section: Arc::from(""),
            blocks: Vec::new(),
            rows: Vec::new(),
        });
        self
    }

    pub fn section(&mut self, section: &str) -> &mut Self {
        match self.hunks.last_mut() {
            Some(hunk) => hunk.section = Arc::from(section),
            None => self.fail(BuildError::NoHunk),
        }
        self
    }

    pub fn context(&mut self, lines: u32) -> &mut Self {
        self.block(BlockKind::Context, lines, lines)
    }

    pub fn change(&mut self, old: u32, new: u32) -> &mut Self {
        self.block(BlockKind::Change, old, new)
    }

    pub fn row(&mut self, row: AlignedRow) -> &mut Self {
        match self.hunks.last_mut() {
            Some(hunk) => hunk.rows.push(row),
            None => self.fail(BuildError::NoHunk),
        }
        self
    }

    pub fn spans(&mut self, side: Side, line: u32, spans: &[SemanticSpan]) -> &mut Self {
        if !spans.is_empty() {
            self.spans[side as usize].push((line, spans.to_vec()));
        }
        self
    }

    pub fn structural(&mut self, structural: bool) -> &mut Self {
        self.structural = structural;
        self
    }

    pub fn outline(&mut self, items: Vec<OutlineItem>) -> &mut Self {
        self.outline = items;
        self
    }

    fn block(&mut self, kind: BlockKind, old: u32, new: u32) -> &mut Self {
        if old == 0 && new == 0 {
            return self;
        }
        let Some(hunk) = self.hunks.last_mut() else {
            self.fail(BuildError::NoHunk);
            return self;
        };
        match hunk.blocks.last_mut() {
            Some(last) if last.0 == kind => {
                last.1 += old;
                last.2 += new;
            }
            _ => hunk.blocks.push((kind, old, new)),
        }
        self
    }

    fn fail(&mut self, error: BuildError) {
        self.error.get_or_insert(error);
    }

    pub fn finish(mut self) -> Result<u32, BuildError> {
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        if self.meta.status == FileStatus::Added && self.old.line_count() > 0 {
            return Err(BuildError::UnexpectedSide(Side::Old));
        }
        if self.meta.status == FileStatus::Deleted && self.new.line_count() > 0 {
            return Err(BuildError::UnexpectedSide(Side::New));
        }
        let partial = self.coverage == SourceCoverage::PatchOnly;
        self.check_lines(partial)?;
        let semantics = self.semantics()?;
        let mut scratch = DiffDocument::default();
        scratch.begin_file(self.meta, partial);
        let (mut old_store, mut new_store) = (0, 0);
        for hunk in &self.hunks {
            let (old_len, new_len) = lengths(hunk);
            let header = (
                hunk.old_line + u32::from(old_len > 0),
                old_len,
                hunk.new_line + u32::from(new_len > 0),
                new_len,
            );
            let stores = if partial {
                (old_store, new_store)
            } else {
                (hunk.old_line, hunk.new_line)
            };
            scratch.push_hunk(header, hunk.section.clone(), stores, &hunk.blocks);
            old_store += old_len;
            new_store += new_len;
        }
        scratch.set_texts(self.old, self.new);
        if let Some(semantics) = &semantics {
            semantics
                .verify(&scratch, 0)
                .map_err(BuildError::Semantics)?;
        }
        scratch.set_semantics(semantics.map(Arc::new));
        scratch.verify_integrity().map_err(BuildError::Integrity)?;
        let index = self.target.file_count();
        self.target.append(scratch);
        Ok(index)
    }

    fn check_lines(&self, partial: bool) -> Result<(), BuildError> {
        let (old, new) = (&self.old, &self.new);
        let (mut old_end, mut new_end) = (0u32, 0u32);
        let (mut old_store, mut new_store) = (0u32, 0u32);
        for (i, hunk) in self.hunks.iter().enumerate() {
            if hunk.blocks.is_empty() {
                return Err(BuildError::EmptyHunk { hunk: i as u32 });
            }
            if hunk.old_line < old_end || hunk.new_line < new_end {
                return Err(BuildError::HunkOrder { hunk: i as u32 });
            }
            if !partial && hunk.old_line - old_end != hunk.new_line - new_end {
                return Err(BuildError::GapLength {
                    old_line: hunk.old_line,
                    new_line: hunk.new_line,
                });
            }
            let (mut o, mut n) = (hunk.old_line, hunk.new_line);
            for &(kind, old_len, new_len) in &hunk.blocks {
                let (os, ns) = if partial {
                    (old_store, new_store)
                } else {
                    (o, n)
                };
                if os + old_len > old.line_count() {
                    return Err(BuildError::OutOfRange {
                        side: Side::Old,
                        line: o + old_len.min(old.line_count().saturating_sub(os)),
                    });
                }
                if ns + new_len > new.line_count() {
                    return Err(BuildError::OutOfRange {
                        side: Side::New,
                        line: n + new_len.min(new.line_count().saturating_sub(ns)),
                    });
                }
                if kind == BlockKind::Context {
                    for k in 0..old_len {
                        if old.line(os + k) != new.line(ns + k) {
                            return Err(BuildError::ContextMismatch {
                                old_line: o + k,
                                new_line: n + k,
                            });
                        }
                    }
                }
                o += old_len;
                n += new_len;
                old_store += old_len;
                new_store += new_len;
            }
            old_end = o;
            new_end = n;
        }
        if partial {
            if old_store != old.line_count() {
                return Err(BuildError::UnusedStore(Side::Old));
            }
            if new_store != new.line_count() {
                return Err(BuildError::UnusedStore(Side::New));
            }
        } else if old.line_count() - old_end != new.line_count() - new_end
            && self.meta.status != FileStatus::Added
            && self.meta.status != FileStatus::Deleted
        {
            return Err(BuildError::GapLength {
                old_line: old_end,
                new_line: new_end,
            });
        }
        Ok(())
    }

    fn semantics(&mut self) -> Result<Option<FileSemantics>, BuildError> {
        let mut semantics = FileSemantics {
            structural: self.structural,
            outline: std::mem::take(&mut self.outline),
            ..FileSemantics::default()
        };
        for hunk in &mut self.hunks {
            let start = semantics.rows.len() as u32;
            semantics.rows.append(&mut hunk.rows);
            semantics.hunk_rows.push(start..semantics.rows.len() as u32);
        }
        for side in [Side::Old, Side::New] {
            let lines = &mut self.spans[side as usize];
            lines.sort_by_key(|(line, _)| *line);
            let mut table = LineSpans::default();
            let mut previous = None;
            for (line, spans) in lines.iter() {
                if previous == Some(*line) {
                    return Err(BuildError::DuplicateSpans { side, line: *line });
                }
                previous = Some(*line);
                table.push(*line, spans);
            }
            semantics.spans[side as usize] = table;
        }
        Ok((!semantics.is_empty()).then_some(semantics))
    }
}

fn lengths(hunk: &HunkDraft) -> (u32, u32) {
    hunk.blocks
        .iter()
        .fold((0, 0), |(o, n), &(_, old, new)| (o + old, n + new))
}

pub fn diff_from_changes(
    meta: FileMeta,
    old: TextStore,
    new: TextStore,
    changes: &[Change],
    context: u32,
) -> Result<DiffDocument, BuildError> {
    let (mut old_end, mut new_end) = (0u32, 0u32);
    for (i, change) in changes.iter().enumerate() {
        let ordered = change.old.start >= old_end
            && change.new.start >= new_end
            && change.old.start <= change.old.end
            && change.new.start <= change.new.end
            && change.old.start - old_end == change.new.start - new_end
            && (!change.old.is_empty() || !change.new.is_empty());
        if !ordered {
            return Err(BuildError::ChangeOrder { change: i as u32 });
        }
        old_end = change.old.end;
        new_end = change.new.end;
    }
    if old_end > old.line_count() {
        return Err(BuildError::OutOfRange {
            side: Side::Old,
            line: old.line_count(),
        });
    }
    if new_end > new.line_count() {
        return Err(BuildError::OutOfRange {
            side: Side::New,
            line: new.line_count(),
        });
    }
    let (old_count, new_count) = (old.line_count(), new.line_count());
    let mut builder = DocumentBuilder::new();
    let mut file = builder.file(meta, SourceCoverage::Full, old, new);
    for group in hunk_groups(changes, context) {
        let first = &group[0];
        let last = &group[group.len() - 1];
        let lead = context.min(first.old.start).min(first.new.start);
        let trail = context
            .min(old_count - last.old.end)
            .min(new_count - last.new.end);
        file.hunk(first.old.start - lead, first.new.start - lead);
        file.context(lead);
        for (i, change) in group.iter().enumerate() {
            if i > 0 {
                file.context(change.old.start - group[i - 1].old.end);
            }
            file.change(change.old.len() as u32, change.new.len() as u32);
        }
        file.context(trail);
    }
    file.finish()?;
    Ok(builder.build())
}
