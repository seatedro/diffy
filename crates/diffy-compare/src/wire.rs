//! The versioned display format that non-Rust clients render diffs from.
//!
//! [`file()`] and [`document`] project compare results ([`FileDiff`]) into
//! plain serializable values; [`expand`] produces the rows of a collapsed
//! gap on request. `FORMAT.md` beside this crate's manifest is the
//! client-facing spec of every field.
//!
//! Versioning: [`Document::version`] is [`FORMAT_VERSION`]. It is bumped
//! for any breaking change: removing or renaming a field or enum value,
//! changing a field's type, meaning, or coordinates, or making an optional
//! field required. Adding an optional field, or a value to an enum that
//! clients are told to tolerate (syntax classes), is not breaking and
//! keeps the version.
//!
//! Coordinates: line numbers are one-based. Span offsets are UTF-16 code
//! units within the row's text, the way JavaScript indexes strings, and
//! ranges are half-open. Text is each line as [`TextStore::display_line`]
//! shows it: no newline, no trailing `\r`, invalid UTF-8 bytes as U+FFFD.
//!
//! Rows are layout-neutral: a `modified` row carries the removed line and
//! the added line it became, so split and unified layouts both render
//! without recomputing anything.

use std::borrow::Cow;
use std::cell::OnceCell;
use std::ops::Range;

use diffy_diff::{
    AlignedRow, BlockKind, ChangeIntensity, DiffDocument, FileStatus, ItemChange, Side as DocSide,
    TextStore, inline_diff,
};
use diffy_syntax::{GrammarStore, HighlightKind, HighlightSpan, LanguageId, ScopeTable};
use serde::{Deserialize, Serialize};

use crate::{FileDiff, FileInput};

/// The format version every [`Document`] carries.
pub const FORMAT_VERSION: u32 = 1;

/// Sides longer than this get no syntax spans or scope headers by default.
pub const SYNTAX_MAX_BYTES: usize = 1 << 20;

/// What a projection adds beyond the diff itself.
#[derive(Debug, Clone, Copy)]
pub struct WireOptions<'a> {
    /// Grammars for syntax spans and for hunk headers of files without a
    /// semantic outline. `None` (the default) emits neither.
    pub syntax: Option<&'a GrammarStore>,
    /// Sides longer than this, in bytes, get no syntax spans or scope
    /// headers.
    pub syntax_max_bytes: usize,
}

impl Default for WireOptions<'_> {
    fn default() -> Self {
        Self {
            syntax: None,
            syntax_max_bytes: SYNTAX_MAX_BYTES,
        }
    }
}

// ---- Format ------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    /// [`FORMAT_VERSION`] of the producer.
    pub version: u32,
    pub files: Vec<File>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct File {
    pub status: Status,
    /// `None` for an added file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    /// `None` for a deleted file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_path: Option<String>,
    /// The language tag the path names (`rs`, `ts`, `dockerfile`), as
    /// diffy names grammars; `None` when it names none or the compare
    /// found no grammar for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// A binary file has no hunks or gaps.
    #[serde(default, skip_serializing_if = "is_false")]
    pub binary: bool,
    pub additions: u32,
    pub deletions: u32,
    /// The old side's last line has no newline.
    #[serde(default, skip_serializing_if = "is_false")]
    pub old_missing_newline: bool,
    /// The new side's last line has no newline.
    #[serde(default, skip_serializing_if = "is_false")]
    pub new_missing_newline: bool,
    /// Why the file got a line diff instead of a semantic one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Fallback>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hunks: Vec<Hunk>,
    /// Unchanged lines after the last hunk; for a file without hunks, the
    /// whole file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_after: Option<Gap>,
    /// Items the semantic diff found changed: new-side items, then
    /// old-side ones.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outline: Vec<OutlineItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub moves: Vec<Move>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub renames: Vec<Rename>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
}

impl From<FileStatus> for Status {
    fn from(status: FileStatus) -> Self {
        match status {
            FileStatus::Modified => Self::Modified,
            FileStatus::Added => Self::Added,
            FileStatus::Deleted => Self::Deleted,
            FileStatus::Renamed => Self::Renamed,
            FileStatus::Copied => Self::Copied,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fallback {
    pub kind: FallbackKind,
    /// Set when `kind` is `semantic`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<SemanticReason>,
    /// For people; wording may change between releases.
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackKind {
    NoGrammar,
    GrammarUnavailable,
    Semantic,
    RequestBudget,
}

/// Why the semantic diff gave up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticReason {
    TooLarge,
    ParseErrors,
    ErrorCoverage,
    Unparsed,
    Budget,
    Timeout,
}

impl From<&crate::Fallback> for Fallback {
    fn from(fallback: &crate::Fallback) -> Self {
        use diffy_semantic::Fallback as Semantic;
        let (kind, reason) = match fallback {
            crate::Fallback::NoGrammar => (FallbackKind::NoGrammar, None),
            crate::Fallback::GrammarUnavailable(_) => (FallbackKind::GrammarUnavailable, None),
            crate::Fallback::RequestBudget => (FallbackKind::RequestBudget, None),
            crate::Fallback::Semantic(semantic) => {
                let reason = match semantic {
                    Semantic::TooLarge(_) => SemanticReason::TooLarge,
                    Semantic::ParseErrors(_) => SemanticReason::ParseErrors,
                    Semantic::ErrorCoverage(_) => SemanticReason::ErrorCoverage,
                    Semantic::Unparsed => SemanticReason::Unparsed,
                    Semantic::Budget => SemanticReason::Budget,
                    Semantic::Timeout => SemanticReason::Timeout,
                };
                (FallbackKind::Semantic, Some(reason))
            }
        };
        Self {
            kind,
            reason,
            message: fallback.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hunk {
    /// The `@@ -old_start,old_len +new_start,new_len @@` numbers: a start
    /// is the first line, or the line before an empty side.
    pub old_start: u32,
    pub old_len: u32,
    pub new_start: u32,
    pub new_len: u32,
    /// The item enclosing the hunk's first change (`render_preview`, or a
    /// scope label such as `impl Parser`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    /// Unchanged lines between the previous hunk (or the file start) and
    /// this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_before: Option<Gap>,
    pub rows: Vec<Row>,
}

/// Collapsed unchanged lines, the same on both sides. Also the request
/// for [`expand`]: any sub-range of a gap is a valid `Gap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Gap {
    /// One-based first line on each side.
    pub old_start: u32,
    pub new_start: u32,
    pub len: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Row {
    /// An unchanged line. `syntax` is the new side's.
    Context {
        old_line: u32,
        new_line: u32,
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        syntax: Vec<SyntaxSpan>,
    },
    Removed {
        old: Line,
    },
    Added {
        new: Line,
    },
    /// A removed line beside the added line it became.
    Modified {
        old: Line,
        new: Line,
    },
}

/// One side's line of a changed row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    /// One-based line number on its side.
    pub line: u32,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<ChangeSpan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub syntax: Vec<SyntaxSpan>,
}

/// A changed run of a line. Offsets are UTF-16 code units within the
/// line's text; spans are sorted and disjoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeSpan {
    pub start: u32,
    pub end: u32,
    pub intensity: Intensity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intensity {
    /// Syntax that exists only on this side.
    Novel,
    /// A changed word inside an otherwise matched line.
    NovelWord,
    /// Unchanged syntax inside a changed line.
    UnchangedContext,
    /// An identifier the diff matched as renamed (see [`File::renames`]).
    Renamed,
}

impl From<ChangeIntensity> for Intensity {
    fn from(intensity: ChangeIntensity) -> Self {
        match intensity {
            ChangeIntensity::Novel => Self::Novel,
            ChangeIntensity::NovelWord => Self::NovelWord,
            ChangeIntensity::UnchangedContext => Self::UnchangedContext,
            ChangeIntensity::Renamed => Self::Renamed,
        }
    }
}

/// A highlighted run of a line. Offsets are UTF-16 code units within the
/// line's text; spans are sorted and disjoint, and text outside them is
/// plain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyntaxSpan {
    pub start: u32,
    pub end: u32,
    /// A [`HighlightKind::name`] such as `keyword` or `string`; clients
    /// map it to their theme and treat unknown classes as plain.
    pub class: Cow<'static, str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Old,
    New,
}

impl From<DocSide> for Side {
    fn from(side: DocSide) -> Self {
        match side {
            DocSide::Old => Self::Old,
            DocSide::New => Self::New,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutlineItem {
    pub side: Side,
    /// One-based first line of the item on `side`.
    pub line: u32,
    /// The grammar's item kind, such as `function_item`.
    pub kind: String,
    pub name: String,
    pub change: ItemChangeKind,
    /// Changed lines of an `edited` item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edits: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemChangeKind {
    Added,
    Removed,
    Renamed,
    Moved,
    Edited,
}

/// Code that moved: `old_len` lines from `old_start` became `new_len`
/// lines at `new_start` (one-based).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Move {
    pub old_start: u32,
    pub old_len: u32,
    pub new_start: u32,
    pub new_len: u32,
    /// The item the code left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The item the code arrived in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub into: Option<String>,
    /// The moved item itself, when the code is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
}

/// An identifier renamed throughout the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rename {
    pub old: String,
    pub new: String,
    pub uses: u32,
}

fn is_false(value: &bool) -> bool {
    !*value
}

// ---- Projection --------------------------------------------------------

/// Projects every file; `inputs` and `diffs` pair up by index, and extra
/// entries of the longer slice are ignored.
pub fn document(inputs: &[FileInput], diffs: &[FileDiff], options: &WireOptions<'_>) -> Document {
    Document {
        version: FORMAT_VERSION,
        files: inputs
            .iter()
            .zip(diffs)
            .map(|(input, diff)| file(input, diff, options))
            .collect(),
    }
}

/// Projects one file. Highlighting and scope headers parse each side once;
/// everything else is linear in the rows shown.
pub fn file(input: &FileInput, diff: &FileDiff, options: &WireOptions<'_>) -> File {
    let doc = &diff.document;
    let has_file = doc.file_count() > 0;
    let binary = crate::is_binary(&input.old)
        || crate::is_binary(&input.new)
        || doc.files().meta.first().is_some_and(|meta| meta.binary);
    let (additions, deletions) = if has_file {
        doc.stats(0).known().unwrap_or((0, 0))
    } else {
        (0, 0)
    };
    let language = match diff.fallback {
        Some(crate::Fallback::NoGrammar) => None,
        _ => LanguageId::from_path(input.display_path()).map(|id| id.as_str().to_owned()),
    };
    let mut out = File {
        status: input.status.into(),
        old_path: input.old_path.clone(),
        new_path: input.new_path.clone(),
        language,
        binary,
        additions,
        deletions,
        old_missing_newline: false,
        new_missing_newline: false,
        fallback: diff.fallback.as_ref().map(Fallback::from),
        hunks: Vec::new(),
        gap_after: None,
        outline: Vec::new(),
        moves: Vec::new(),
        renames: Vec::new(),
    };
    if binary || !has_file {
        return out;
    }
    let facts = doc.facts(0);
    out.old_missing_newline = facts.old_missing_newline;
    out.new_missing_newline = facts.new_missing_newline;
    if let Some(semantics) = doc.semantics(0) {
        out.outline = semantics.outline().iter().map(outline_item).collect();
        out.moves = semantics.moves().iter().map(wire_move).collect();
        out.renames = semantics
            .renames()
            .iter()
            .map(|rename| Rename {
                old: rename.old.to_string(),
                new: rename.new.to_string(),
                uses: rename.uses,
            })
            .collect();
    }
    Projector::new(doc, input, options).hunks(&mut out);
    out
}

/// Context rows for `gap` (or any sub-range of a gap) of `input`, with
/// the new side's syntax spans when `options` asks for them. Highlighting
/// parses the whole new side so lexical state is right. Rows stop early
/// where the range leaves either side or the sides differ, as a stale
/// request would.
pub fn expand(input: &FileInput, gap: &Gap, options: &WireOptions<'_>) -> Vec<Row> {
    if gap.len == 0
        || gap.old_start == 0
        || gap.new_start == 0
        || crate::is_binary(&input.old)
        || crate::is_binary(&input.new)
    {
        return Vec::new();
    }
    let old = TextStore::from_bytes(&input.old);
    let new = TextStore::from_bytes(&input.new);
    let (old_first, new_first) = (gap.old_start - 1, gap.new_start - 1);
    let len = gap
        .len
        .min(old.line_count().saturating_sub(old_first))
        .min(new.line_count().saturating_sub(new_first));
    let new_path = input.new_path.as_deref().or(input.old_path.as_deref());
    let syntax = highlights(options, new_path, &new);
    let mut rows = Vec::with_capacity(len as usize);
    for k in 0..len {
        let (o, n) = (old_first + k, new_first + k);
        if old.line(o) != new.line(n) {
            break;
        }
        rows.push(context_row(&new, &syntax, n, o + 1, n + 1));
    }
    rows
}

fn outline_item(item: &diffy_diff::OutlineItem) -> OutlineItem {
    let (change, edits) = match item.change {
        ItemChange::Added => (ItemChangeKind::Added, None),
        ItemChange::Removed => (ItemChangeKind::Removed, None),
        ItemChange::Renamed => (ItemChangeKind::Renamed, None),
        ItemChange::Moved => (ItemChangeKind::Moved, None),
        ItemChange::Edited(lines) => (ItemChangeKind::Edited, Some(lines)),
    };
    OutlineItem {
        side: item.side.into(),
        line: item.line + 1,
        kind: item.kind.to_string(),
        name: item.name.to_string(),
        change,
        edits,
    }
}

fn wire_move(moved: &diffy_diff::Move) -> Move {
    let name = |name: &Option<std::sync::Arc<str>>| name.as_deref().map(str::to_owned);
    Move {
        old_start: moved.old_line + 1,
        old_len: moved.old_lines,
        new_start: moved.new_line + 1,
        new_len: moved.lines,
        from: name(&moved.from),
        into: name(&moved.into),
        item: name(&moved.item),
    }
}

/// Builds the hunks of file 0 of a one-file document.
struct Projector<'a> {
    doc: &'a DiffDocument,
    input: &'a FileInput,
    options: &'a WireOptions<'a>,
    /// The stores hold only the lines the patch shows: no gaps, and no
    /// syntax (fragments would parse out of context).
    partial: bool,
    texts: [&'a TextStore; 2],
    syntax: [Vec<HighlightSpan>; 2],
    scopes: [OnceCell<ScopeTable>; 2],
}

impl<'a> Projector<'a> {
    fn new(doc: &'a DiffDocument, input: &'a FileInput, options: &'a WireOptions<'a>) -> Self {
        let partial = doc.files().partial[0];
        let texts = [doc.text(0, DocSide::Old), doc.text(0, DocSide::New)];
        let syntax = if partial {
            [Vec::new(), Vec::new()]
        } else {
            [DocSide::Old, DocSide::New]
                .map(|side| highlights(options, side_path(input, side), texts[side as usize]))
        };
        Self {
            doc,
            input,
            options,
            partial,
            texts,
            syntax,
            scopes: [OnceCell::new(), OnceCell::new()],
        }
    }

    fn hunks(&self, out: &mut File) {
        let (doc, partial) = (self.doc, self.partial);
        let (h, b) = (doc.hunks(), doc.blocks());
        let structural = doc.prefers_structural(0);
        let mut steps = Vec::new();
        // Zero-based ends of the previous hunk on each side.
        let (mut old_end, mut new_end) = (0u32, 0u32);
        for hunk in doc.files().hunks[0].clone() {
            let hi = hunk as usize;
            let old_lines = doc.hunk_lines(hunk, DocSide::Old);
            let new_lines = doc.hunk_lines(hunk, DocSide::New);
            let gap_before = if partial {
                None
            } else {
                gap(
                    (old_end, new_end),
                    old_lines.start.saturating_sub(old_end),
                    new_lines.start.saturating_sub(new_end),
                )
            };
            let rows_hint = h.old_len[hi].max(h.new_len[hi]) as usize;
            let mut rows = Vec::with_capacity(rows_hint);
            let aligned = if structural {
                doc.aligned_rows(hunk)
            } else {
                &[]
            };
            for block in h.blocks[hi].clone() {
                let bi = block as usize;
                let at = BlockAt {
                    old_store: b.old_store[bi],
                    new_store: b.new_store[bi],
                    old_line: b.old_line[bi] - 1,
                    new_line: b.new_line[bi] - 1,
                };
                match b.kind[bi] {
                    BlockKind::Context => {
                        for k in 0..b.old_len[bi] {
                            rows.push(context_row(
                                self.texts[1],
                                &self.syntax[1],
                                at.new_store + k,
                                at.old_line + k + 1,
                                at.new_line + k + 1,
                            ));
                        }
                    }
                    BlockKind::Change => {
                        let lens = (b.old_len[bi], b.new_len[bi]);
                        steps.clear();
                        if aligned.is_empty() || !structural_steps(aligned, &at, lens, &mut steps) {
                            steps.clear();
                            positional_steps(lens, &mut steps);
                        }
                        for &step in &steps {
                            rows.push(self.change_row(&at, step));
                        }
                    }
                }
            }
            out.hunks.push(Hunk {
                old_start: h.old_start[hi],
                old_len: h.old_len[hi],
                new_start: h.new_start[hi],
                new_len: h.new_len[hi],
                header: self.header(hunk),
                gap_before,
                rows,
            });
            old_end = old_lines.end;
            new_end = new_lines.end;
        }
        if !partial {
            out.gap_after = gap(
                (old_end, new_end),
                self.texts[0].line_count().saturating_sub(old_end),
                self.texts[1].line_count().saturating_sub(new_end),
            );
        }
    }

    /// The row of one step of a change block; offsets are within the block.
    fn change_row(&self, at: &BlockAt, step: Step) -> Row {
        let line = |side: DocSide, offset: u32| match side {
            DocSide::Old => (at.old_store + offset, at.old_line + offset),
            DocSide::New => (at.new_store + offset, at.new_line + offset),
        };
        match step {
            Step::Paired(o, n) => {
                let (old, new) = (line(DocSide::Old, o), line(DocSide::New, n));
                let old_spans = self.doc.semantic_spans(0, DocSide::Old, old.1);
                let new_spans = self.doc.semantic_spans(0, DocSide::New, new.1);
                if old_spans.is_empty() && new_spans.is_empty() {
                    // A line-diff pair, or lines the semantic diff left
                    // unmarked: changed words come from the word diff.
                    let old_text = self.display(DocSide::Old, old.0);
                    let new_text = self.display(DocSide::New, new.0);
                    let words = inline_diff(old_text, new_text);
                    Row::Modified {
                        old: self.line(DocSide::Old, old, word_spans(&words.old)),
                        new: self.line(DocSide::New, new, word_spans(&words.new)),
                    }
                } else {
                    Row::Modified {
                        old: self.line(DocSide::Old, old, semantic(old_spans)),
                        new: self.line(DocSide::New, new, semantic(new_spans)),
                    }
                }
            }
            Step::Removed(o) => {
                let old = line(DocSide::Old, o);
                let spans = semantic(self.doc.semantic_spans(0, DocSide::Old, old.1));
                Row::Removed {
                    old: self.line(DocSide::Old, old, spans),
                }
            }
            Step::Added(n) => {
                let new = line(DocSide::New, n);
                let spans = semantic(self.doc.semantic_spans(0, DocSide::New, new.1));
                Row::Added {
                    new: self.line(DocSide::New, new, spans),
                }
            }
        }
    }

    fn display(&self, side: DocSide, store: u32) -> &str {
        self.texts[side as usize].display_line(store).unwrap_or("")
    }

    /// One side's line: `(store index, zero-based line)`, and its changed
    /// byte ranges within the display text.
    fn line(
        &self,
        side: DocSide,
        (store, line): (u32, u32),
        changes: impl IntoIterator<Item = (Range<usize>, Intensity)>,
    ) -> Line {
        let texts = self.texts[side as usize];
        let text = texts.display_line(store).unwrap_or("");
        let mut units = Utf16::new(text);
        let changes = changes
            .into_iter()
            .filter_map(|(range, intensity)| {
                let (start, end) = units.range(range)?;
                Some(ChangeSpan {
                    start,
                    end,
                    intensity,
                })
            })
            .collect();
        let syntax = syntax_spans(texts, &self.syntax[side as usize], store, &mut units);
        Line {
            line: line + 1,
            text: text.to_owned(),
            changes,
            syntax,
        }
    }

    /// The item enclosing the hunk's first change: the nearest changed
    /// item of the semantic outline starting at or before it, else the
    /// innermost scope of the grammar's scope table.
    fn header(&self, hunk: u32) -> Option<String> {
        let (h, b) = (self.doc.hunks(), self.doc.blocks());
        let hi = hunk as usize;
        let first_change = h.blocks[hi]
            .clone()
            .map(|block| block as usize)
            .find(|&bi| b.kind[bi] == BlockKind::Change);
        // Zero-based anchors on each side, and the side the change shows.
        let (old, new, side) = match first_change {
            Some(bi) => (
                b.old_line[bi] - 1,
                b.new_line[bi] - 1,
                if b.new_len[bi] > 0 {
                    DocSide::New
                } else {
                    DocSide::Old
                },
            ),
            None => (
                h.old_start[hi].saturating_sub(1),
                h.new_start[hi].saturating_sub(1),
                DocSide::New,
            ),
        };
        if let Some(semantics) = self.doc.semantics(0) {
            // Items list no end line, so the nearest start before the
            // change stands in for the enclosing item. Edited items sit on
            // the new side, removed ones on the old; ties go to new.
            let nearest = semantics
                .outline()
                .iter()
                .filter_map(|item| {
                    let anchor = match item.side {
                        DocSide::Old => old,
                        DocSide::New => new,
                    };
                    (item.line <= anchor).then(|| (anchor - item.line, item.side, item))
                })
                .min_by_key(|&(distance, side, _)| (distance, side == DocSide::Old));
            if let Some((_, _, item)) = nearest {
                return Some(item.name.to_string());
            }
        }
        let line = match side {
            DocSide::Old => old,
            DocSide::New => new,
        };
        // Sorted by start, longer first, so the last span containing the
        // line is the innermost.
        self.scopes(side)
            .spans()
            .iter()
            .rfind(|span| span.lines.contains(&line))
            .map(|span| span.label.to_string())
    }

    fn scopes(&self, side: DocSide) -> &ScopeTable {
        self.scopes[side as usize].get_or_init(|| {
            let text = self.texts[side as usize].as_str();
            let language = side_path(self.input, side).and_then(LanguageId::from_path);
            match (self.options.syntax, language) {
                (Some(grammars), Some(language))
                    if !self.partial && text.len() <= self.options.syntax_max_bytes =>
                {
                    diffy_syntax::scope_table(grammars, &language, text)
                }
                _ => ScopeTable::default(),
            }
        })
    }
}

/// Where a block's first line sits: store indices and zero-based lines.
struct BlockAt {
    old_store: u32,
    new_store: u32,
    old_line: u32,
    new_line: u32,
}

/// The gap after zero-based ends `(old, new)`; the sides agree on its
/// length in a valid document, the shorter wins otherwise.
fn gap((old, new): (u32, u32), old_len: u32, new_len: u32) -> Option<Gap> {
    let len = old_len.min(new_len);
    (len > 0).then_some(Gap {
        old_start: old + 1,
        new_start: new + 1,
        len,
    })
}

fn side_path(input: &FileInput, side: DocSide) -> Option<&str> {
    let (own, other) = match side {
        DocSide::Old => (&input.old_path, &input.new_path),
        DocSide::New => (&input.new_path, &input.old_path),
    };
    own.as_deref().or(other.as_deref())
}

/// A row of a change block; offsets are within the block.
#[derive(Debug, Clone, Copy)]
enum Step {
    Paired(u32, u32),
    Removed(u32),
    Added(u32),
}

/// Pairs a change block's lines as the semantic diff aligned them, when
/// its rows cover the block in order; offsets within the block.
fn structural_steps(
    rows: &[AlignedRow],
    at: &BlockAt,
    (old_len, new_len): (u32, u32),
    steps: &mut Vec<Step>,
) -> bool {
    let offset = |line: Option<u32>, first: u32, len: u32| {
        line.and_then(|line| line.checked_sub(first))
            .filter(|&offset| offset < len)
    };
    let (mut o, mut n) = (0, 0);
    for row in rows {
        let ro = offset(row.old, at.old_line, old_len);
        let rn = offset(row.new, at.new_line, new_len);
        if ro.is_some_and(|i| i != o) || rn.is_some_and(|i| i != n) {
            return false;
        }
        steps.push(match (ro, rn) {
            (Some(a), Some(b)) => Step::Paired(a, b),
            (Some(a), None) => Step::Removed(a),
            (None, Some(b)) => Step::Added(b),
            (None, None) => continue,
        });
        o += u32::from(ro.is_some());
        n += u32::from(rn.is_some());
    }
    o == old_len && n == new_len
}

/// The k-th removed line beside the k-th added line, then the rest.
fn positional_steps((old_len, new_len): (u32, u32), steps: &mut Vec<Step>) {
    let paired = old_len.min(new_len);
    steps.extend((0..paired).map(|k| Step::Paired(k, k)));
    steps.extend((paired..old_len).map(Step::Removed));
    steps.extend((paired..new_len).map(Step::Added));
}

fn word_spans(ranges: &[Range<u32>]) -> impl Iterator<Item = (Range<usize>, Intensity)> + '_ {
    ranges.iter().map(|range| {
        (
            range.start as usize..range.end as usize,
            Intensity::NovelWord,
        )
    })
}

fn semantic(
    spans: &[diffy_diff::SemanticSpan],
) -> impl Iterator<Item = (Range<usize>, Intensity)> + '_ {
    spans.iter().map(|span| {
        (
            span.start as usize..span.end as usize,
            span.intensity.into(),
        )
    })
}

/// A context row of store line `store` of `text`, with its syntax.
fn context_row(
    text: &TextStore,
    syntax: &[HighlightSpan],
    store: u32,
    old_line: u32,
    new_line: u32,
) -> Row {
    let line = text.display_line(store).unwrap_or("");
    let mut units = Utf16::new(line);
    Row::Context {
        old_line,
        new_line,
        text: line.to_owned(),
        syntax: syntax_spans(text, syntax, store, &mut units),
    }
}

/// Highlights a whole side once, when `options` asks for syntax and the
/// side is within the size cap.
fn highlights(
    options: &WireOptions<'_>,
    path: Option<&str>,
    text: &TextStore,
) -> Vec<HighlightSpan> {
    let source = text.as_str();
    match (options.syntax, path.and_then(LanguageId::from_path)) {
        (Some(grammars), Some(language))
            if !source.is_empty() && source.len() <= options.syntax_max_bytes =>
        {
            diffy_syntax::highlight(grammars, &language, source)
        }
        _ => Vec::new(),
    }
}

/// The spans of whole-text `syntax` on store line `store`, clipped to its
/// display text and converted to UTF-16 offsets within it.
fn syntax_spans(
    text: &TextStore,
    syntax: &[HighlightSpan],
    store: u32,
    units: &mut Utf16<'_>,
) -> Vec<SyntaxSpan> {
    let Some(line) = text.display_range(store) else {
        return Vec::new();
    };
    if syntax.is_empty() || line.is_empty() {
        return Vec::new();
    }
    // Spans are sorted and disjoint, so their ends ascend too.
    let first = syntax.partition_point(|span| span.range().end <= line.start);
    syntax[first..]
        .iter()
        .take_while(|span| (span.offset as usize) < line.end)
        .filter(|span| span.kind != HighlightKind::Normal)
        .filter_map(|span| {
            let range = span.range();
            let start = range.start.max(line.start) - line.start;
            let end = range.end.min(line.end) - line.start;
            let (start, end) = units.range(start..end)?;
            Some(SyntaxSpan {
                start,
                end,
                class: Cow::Borrowed(span.kind.name()),
            })
        })
        .collect()
}

/// Converts byte offsets within a line to UTF-16 offsets, walking forward
/// from the previous offset; cheap for the ascending offsets of sorted
/// spans, and free for ASCII lines.
struct Utf16<'t> {
    text: &'t str,
    ascii: bool,
    byte: usize,
    unit: u32,
}

impl<'t> Utf16<'t> {
    fn new(text: &'t str) -> Self {
        Self {
            text,
            ascii: text.is_ascii(),
            byte: 0,
            unit: 0,
        }
    }

    /// The UTF-16 offset of byte `at`, clamped to the text and moved back
    /// to a char boundary.
    fn at(&mut self, at: usize) -> u32 {
        let mut at = at.min(self.text.len());
        if self.ascii {
            return at as u32;
        }
        while !self.text.is_char_boundary(at) {
            at -= 1;
        }
        if at < self.byte {
            self.byte = 0;
            self.unit = 0;
        }
        self.unit += self.text[self.byte..at].encode_utf16().count() as u32;
        self.byte = at;
        self.unit
    }

    /// A byte range in UTF-16 offsets; `None` when it is empty.
    fn range(&mut self, range: Range<usize>) -> Option<(u32, u32)> {
        let start = self.at(range.start);
        let end = self.at(range.end);
        (start < end).then_some((start, end))
    }
}

#[cfg(test)]
mod tests;
