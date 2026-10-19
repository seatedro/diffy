use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use diffy_diff::{
    AlignedRow, ChangeIntensity, DiffDocument, DocumentBuilder, FileMeta, FileStatus, SemanticSpan,
    Side, SourceCoverage, TextStore,
};
use diffy_semantic::{Limits, Row};
use diffy_syntax::{GrammarStore, LanguageId, legacy_pack};
use rayon::prelude::*;
use vendored_difftastic::{
    ChangeIntensity as DftIntensity, ChangeSpan, DiffRequest, DiffStatus, SemanticDiffResult,
};

use crate::core::compare::git::{
    binary_document, document_from_bytes, is_binary_bytes, line_changes, resolve_compare_refs,
};
use crate::core::compare::output::{CompareEntry, CompareOutput};
use crate::core::compare::progress::{ComparePhase, ProgressSink};
use crate::core::compare::spec::CompareSpec;
use crate::core::compare::stats::COMPARE_SUMMARY_FILE_LIMIT;
use crate::core::error::{DiffyError, Result};
use crate::core::vcs::git::{GitService, INDEX_REF, StatusItem, StatusScope, WORKDIR_REF};

const LOADING_FILE_EMIT_STRIDE: u32 = 16;
const MAX_WORKERS: usize = 4;
const MIN_FILES_FOR_PARALLEL: usize = 4;

#[derive(Debug, Clone)]
pub struct ChangedPath {
    pub status: FileStatus,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub old_content: Vec<u8>,
    pub new_content: Vec<u8>,
}

impl ChangedPath {
    fn display_path(&self) -> &str {
        self.new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .unwrap_or_default()
    }

    fn meta(&self) -> FileMeta {
        FileMeta {
            old_path: self.old_path.as_deref().map(Arc::from),
            new_path: self.new_path.as_deref().map(Arc::from),
            status: self.status,
            ..FileMeta::default()
        }
    }
}

pub fn status_from_label(label: &str) -> FileStatus {
    match label.as_bytes().first() {
        Some(b'A') => FileStatus::Added,
        Some(b'D') => FileStatus::Deleted,
        Some(b'R') => FileStatus::Renamed,
        Some(b'C') => FileStatus::Copied,
        _ => FileStatus::Modified,
    }
}

pub fn compare(
    spec: &CompareSpec,
    git: &GitService,
    reporter: Option<&dyn ProgressSink>,
) -> Result<CompareOutput> {
    let (left, right) = resolve_compare_refs(spec, git)?;
    if let Some(r) = reporter {
        r.phase(ComparePhase::EnumeratingChanges);
    }
    let entries = git.diff_name_status(&left, &right, None)?;
    if right != WORKDIR_REF && entries.len() > COMPARE_SUMMARY_FILE_LIMIT {
        return Ok(CompareOutput::deferred(
            entries
                .iter()
                .map(|(status, old, new)| {
                    CompareEntry::deferred(
                        old.as_deref(),
                        new.as_deref(),
                        status_from_label(status),
                    )
                })
                .collect(),
        ));
    }
    let changed = read_changed_paths(git, &left, &right, entries);
    compare_changed_paths(changed, reporter)
}

pub fn compare_path(spec: &CompareSpec, path: &str, git: &GitService) -> Result<CompareOutput> {
    let (left, right) = resolve_compare_refs(spec, git)?;
    let entries = git.diff_name_status(&left, &right, Some(path))?;
    let changed = read_changed_paths(git, &left, &right, entries);
    compare_changed_paths(changed, None)
}

pub fn compare_status_item(item: &StatusItem, git: &GitService) -> Result<CompareOutput> {
    let (old_ref, new_ref) = match item.scope {
        StatusScope::Staged => (Some("HEAD"), INDEX_REF),
        StatusScope::Unstaged => (Some(INDEX_REF), WORKDIR_REF),
        StatusScope::Untracked => (None, WORKDIR_REF),
    };
    let old_path = item.old_path.as_deref().unwrap_or(&item.path);
    let old_content =
        old_ref.and_then(|reference| git.read_file_bytes_at(reference, old_path).ok());
    let deleted = item.status == "D" && item.scope != StatusScope::Untracked;
    let new_content = (!deleted)
        .then(|| git.read_file_bytes_at(new_ref, &item.path).ok())
        .flatten();
    let status = match (&old_content, &new_content) {
        (None, _) => FileStatus::Added,
        (_, None) => FileStatus::Deleted,
        _ => status_from_label(&item.status),
    };
    let changed = ChangedPath {
        status,
        old_path: old_content.as_ref().map(|_| old_path.to_owned()),
        new_path: new_content.as_ref().map(|_| item.path.clone()),
        old_content: old_content.unwrap_or_default(),
        new_content: new_content.unwrap_or_default(),
    };
    compare_changed_paths(vec![changed], None)
}

pub fn compare_text(left: &str, right: &str, display_path: &str) -> Result<CompareOutput> {
    if left == right {
        return Ok(CompareOutput::default());
    }
    let status = if left.is_empty() {
        FileStatus::Added
    } else if right.is_empty() {
        FileStatus::Deleted
    } else {
        FileStatus::Modified
    };
    let changed = ChangedPath {
        status,
        old_path: (status != FileStatus::Added).then(|| display_path.to_owned()),
        new_path: (status != FileStatus::Deleted).then(|| display_path.to_owned()),
        old_content: left.as_bytes().to_vec(),
        new_content: right.as_bytes().to_vec(),
    };
    compare_changed_paths(vec![changed], None)
}

pub fn compare_changed_paths(
    changed: Vec<ChangedPath>,
    reporter: Option<&dyn ProgressSink>,
) -> Result<CompareOutput> {
    let files_total = changed.len() as u32;
    if let Some(r) = reporter {
        r.phase(ComparePhase::LoadingFiles {
            files_seen: 0,
            files_total,
        });
    }
    let progress = AtomicU32::new(0);
    let load = |changed: &ChangedPath| {
        let doc = document_from_changed(changed)?;
        let seen = progress.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        if let Some(r) = reporter
            && (seen.is_multiple_of(LOADING_FILE_EMIT_STRIDE) || seen == files_total)
        {
            r.phase(ComparePhase::LoadingFiles {
                files_seen: seen,
                files_total,
            });
        }
        Ok(Arc::new(doc))
    };
    let workers = worker_count(changed.len());
    let docs = if workers == 1 {
        changed.iter().map(load).collect::<Result<Vec<_>>>()?
    } else {
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .thread_name(|index| format!("diffy-difftastic-{index}"))
            .build()
            .map_err(|error| {
                DiffyError::General(format!("difftastic worker setup failed: {error}"))
            })?
            .install(|| changed.par_iter().map(load).collect::<Result<Vec<_>>>())?
    };
    let mut output = CompareOutput::default();
    for doc in docs {
        output.push_document(doc);
    }
    Ok(output)
}

pub fn document_from_changed(changed: &ChangedPath) -> Result<DiffDocument> {
    let meta = changed.meta();
    if is_binary_bytes(&changed.old_content) || is_binary_bytes(&changed.new_content) {
        return binary_document(meta);
    }
    let old = TextStore::from_bytes(&changed.old_content);
    let new = TextStore::from_bytes(&changed.new_content);
    if changed.status == FileStatus::Added || changed.old_path.is_none() {
        return whole_file(meta, old, new, Side::New);
    }
    if changed.status == FileStatus::Deleted || changed.new_path.is_none() {
        return whole_file(meta, old, new, Side::Old);
    }
    if changed.old_content == changed.new_content {
        return build(DocumentBuilder::new(), meta, old, new, |_| {});
    }
    let result = vendored_difftastic::diff_bytes_semantic(DiffRequest {
        display_path: changed.display_path(),
        lhs_path: changed.old_path.as_deref().map(Path::new),
        rhs_path: changed.new_path.as_deref().map(Path::new),
        lhs_bytes: old.as_str().as_bytes(),
        rhs_bytes: new.as_str().as_bytes(),
    })
    .map_err(|error| DiffyError::General(format!("difftastic failed: {error}")))?;
    log_line_fallback(changed, &result);
    match result.status {
        DiffStatus::Binary => binary_document(meta),
        DiffStatus::Changed if !result.chunks.is_empty() => {
            document_from_semantic(meta, old, new, &result)
        }
        _ => document_from_bytes(meta, &changed.old_content, &changed.new_content),
    }
}

fn whole_file(meta: FileMeta, old: TextStore, new: TextStore, side: Side) -> Result<DiffDocument> {
    let lines = match side {
        Side::Old => old.line_count(),
        Side::New => new.line_count(),
    };
    let (old, new) = match side {
        Side::Old => (old, TextStore::default()),
        Side::New => (TextStore::default(), new),
    };
    build(DocumentBuilder::new(), meta, old, new, |file| {
        if lines > 0 {
            match side {
                Side::Old => file.hunk(0, 0).change(lines, 0),
                Side::New => file.hunk(0, 0).change(0, lines),
            };
        }
    })
}

fn build(
    mut builder: DocumentBuilder,
    meta: FileMeta,
    old: TextStore,
    new: TextStore,
    shape: impl FnOnce(&mut diffy_diff::FileBuilder<'_>),
) -> Result<DiffDocument> {
    let mut file = builder.file(meta, SourceCoverage::Full, old, new);
    shape(&mut file);
    file.finish()
        .map_err(|error| DiffyError::Parse(error.to_string()))?;
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
    old: std::ops::Range<u32>,
    new: std::ops::Range<u32>,
    segments: std::ops::Range<usize>,
}

pub(crate) fn document_from_semantic(
    meta: FileMeta,
    old: TextStore,
    new: TextStore,
    result: &SemanticDiffResult,
) -> Result<DiffDocument> {
    let chunks = sanitize(result, &old, &new);
    let structural =
        result.line_fallback_reason.is_none() && line_fallback_reason(&result.language).is_none();
    document_from_rows(meta, old, new, chunks, structural)
}

pub fn document_from_native(
    changed: &ChangedPath,
    grammars: &GrammarStore,
) -> Result<(DiffDocument, Option<String>)> {
    document_from_native_with(changed, |name| {
        LanguageId::from_fence(name).and_then(|id| grammars.tree_sitter_language(&id))
    })
}

pub fn document_from_native_with(
    changed: &ChangedPath,
    grammar: impl FnOnce(&str) -> Option<diffy_semantic::Language>,
) -> Result<(DiffDocument, Option<String>)> {
    let meta = changed.meta();
    if is_binary_bytes(&changed.old_content) || is_binary_bytes(&changed.new_content) {
        return Ok((binary_document(meta)?, None));
    }
    let old = TextStore::from_bytes(&changed.old_content);
    let new = TextStore::from_bytes(&changed.new_content);
    if changed.status == FileStatus::Added || changed.old_path.is_none() {
        return Ok((whole_file(meta, old, new, Side::New)?, None));
    }
    if changed.status == FileStatus::Deleted || changed.new_path.is_none() {
        return Ok((whole_file(meta, old, new, Side::Old)?, None));
    }
    if changed.old_content == changed.new_content {
        return Ok((build(DocumentBuilder::new(), meta, old, new, |_| {})?, None));
    }
    let line_diff = |reason: String| {
        document_from_bytes(meta.clone(), &changed.old_content, &changed.new_content)
            .map(|doc| (doc, Some(reason)))
    };
    let Some(language) = legacy_pack::language_for_path(changed.display_path()) else {
        return line_diff("no grammar for this file".to_owned());
    };
    let (Some(grammar), Some(config)) = (
        grammar(language.name),
        diffy_semantic::config(language.name),
    ) else {
        return line_diff(format!("{} grammar unavailable", language.name));
    };
    match diffy_semantic::diff(
        &grammar,
        config,
        old.as_str(),
        new.as_str(),
        &Limits::default(),
    ) {
        Ok(chunks) => {
            let chunks = clean_rows(chunks, &old, &new);
            Ok((document_from_rows(meta, old, new, chunks, true)?, None))
        }
        Err(fallback) => line_diff(fallback.to_string()),
    }
}

fn clean_rows(chunks: Vec<Vec<Row>>, old: &TextStore, new: &TextStore) -> Vec<Vec<Row>> {
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

fn document_from_rows(
    meta: FileMeta,
    old: TextStore,
    new: TextStore,
    chunks: Vec<Vec<Row>>,
    structural: bool,
) -> Result<DiffDocument> {
    let segments = canonical_segments(&chunks, &old, &new);
    let hunks = hunk_ranges(&segments);
    let mut rows: Vec<Vec<AlignedRow>> = hunks.iter().map(|_| Vec::new()).collect();
    let mut spans: [Vec<(u32, Vec<SemanticSpan>)>; 2] = [Vec::new(), Vec::new()];
    let mut seen = [
        std::collections::HashSet::new(),
        std::collections::HashSet::new(),
    ];
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
    build(DocumentBuilder::new(), meta, old, new, |file| {
        file.structural(structural);
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

fn sanitize(result: &SemanticDiffResult, old: &TextStore, new: &TextStore) -> Vec<Vec<Row>> {
    let side_spans = |store: &TextStore, line: Option<u32>, spans: &[ChangeSpan]| {
        line.and_then(|line| store.display_line(line))
            .map(|text| sanitize_spans(spans, text))
            .unwrap_or_default()
    };
    result
        .chunks
        .iter()
        .map(|chunk| {
            chunk
                .lines
                .iter()
                .map(|line| {
                    let old_line = line.lhs_line.filter(|&l| l < old.line_count());
                    let new_line = line.rhs_line.filter(|&l| l < new.line_count());
                    Row {
                        old: old_line,
                        new: new_line,
                        spans: [
                            side_spans(old, old_line, &line.lhs_changes),
                            side_spans(new, new_line, &line.rhs_changes),
                        ],
                    }
                })
                .collect()
        })
        .collect()
}

fn sanitize_spans(spans: &[ChangeSpan], text: &str) -> Vec<SemanticSpan> {
    let spans = spans
        .iter()
        .map(|span| SemanticSpan {
            start: span.start_col,
            end: span.end_col,
            intensity: intensity(span.intensity),
        })
        .collect();
    clean_spans(spans, text)
}

fn clean_spans(spans: Vec<SemanticSpan>, text: &str) -> Vec<SemanticSpan> {
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

fn intensity(intensity: DftIntensity) -> ChangeIntensity {
    match intensity {
        DftIntensity::Novel => ChangeIntensity::Novel,
        DftIntensity::NovelWord => ChangeIntensity::NovelWord,
        DftIntensity::UnchangedContext => ChangeIntensity::UnchangedContext,
    }
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
    old_lines: std::ops::Range<u32>,
    new_lines: std::ops::Range<u32>,
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

fn lines_bytes(store: &TextStore, lines: std::ops::Range<u32>) -> &[u8] {
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

fn worker_count(files: usize) -> usize {
    if files < MIN_FILES_FOR_PARALLEL {
        return 1;
    }
    let available = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    files.min(available).clamp(1, MAX_WORKERS)
}

fn line_fallback_reason(language: &str) -> Option<&str> {
    language
        .strip_prefix("Text (")
        .and_then(|reason| reason.strip_suffix(')'))
}

fn log_line_fallback(changed: &ChangedPath, result: &SemanticDiffResult) {
    if let Some(reason) = result
        .line_fallback_reason
        .as_deref()
        .or_else(|| line_fallback_reason(&result.language))
    {
        tracing::info!(
            target: "diffy::difftastic",
            path = %changed.display_path(),
            reason,
            chunks = result.chunks.len(),
            old_bytes = changed.old_content.len(),
            new_bytes = changed.new_content.len(),
            "difftastic semantic diff fell back to line diff"
        );
    }
}

fn read_changed_paths(
    git: &GitService,
    left: &str,
    right: &str,
    entries: Vec<(String, Option<String>, Option<String>)>,
) -> Vec<ChangedPath> {
    let old_paths = entries.iter().filter_map(|(_, old, _)| old.as_deref());
    let mut old_contents = git.read_file_bytes_batch_at(left, old_paths).into_iter();
    let new_paths = entries.iter().filter_map(|(_, _, new)| new.as_deref());
    let mut new_contents = git.read_file_bytes_batch_at(right, new_paths).into_iter();
    entries
        .into_iter()
        .map(|(status, old_path, new_path)| {
            let old_content = old_path
                .as_ref()
                .and_then(|_| old_contents.next().flatten());
            let new_content = new_path
                .as_ref()
                .and_then(|_| new_contents.next().flatten());
            ChangedPath {
                status: status_from_label(&status),
                old_path,
                new_path,
                old_content: old_content.unwrap_or_default(),
                new_content: new_content.unwrap_or_default(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use diffy_diff::{
        BlockKind, ChangeIntensity as Intensity, FileMeta, FileStatus, Side, TextStore,
    };
    use vendored_difftastic::{
        ChangeIntensity as DftIntensity, ChangeSpan, DiffStatus, HighlightKind, SemanticChunk,
        SemanticDiffResult, SemanticLine,
    };

    use super::{
        ChangedPath, compare, compare_path, document_from_changed, document_from_semantic,
    };
    use crate::core::compare::service::tests::commit_file;
    use crate::core::compare::spec::{CompareMode, CompareSpec, LayoutMode, RendererKind};
    use crate::core::vcs::git::{GitService, WORKDIR_REF};

    fn span(start: u32, end: u32, intensity: DftIntensity) -> ChangeSpan {
        ChangeSpan {
            start_col: start,
            end_col: end,
            highlight: HighlightKind::Normal,
            intensity,
        }
    }

    fn line(
        lhs: Option<u32>,
        rhs: Option<u32>,
        lhs_changes: Vec<ChangeSpan>,
        rhs_changes: Vec<ChangeSpan>,
    ) -> SemanticLine {
        SemanticLine {
            lhs_line: lhs,
            rhs_line: rhs,
            lhs_changes,
            rhs_changes,
        }
    }

    fn result(chunks: Vec<Vec<SemanticLine>>) -> SemanticDiffResult {
        SemanticDiffResult {
            status: DiffStatus::Changed,
            language: "Rust".to_owned(),
            line_fallback_reason: None,
            aligned_lines: Vec::new(),
            chunks: chunks
                .into_iter()
                .map(|lines| SemanticChunk { lines })
                .collect(),
        }
    }

    fn meta() -> FileMeta {
        FileMeta {
            old_path: Some("src/lib.rs".into()),
            new_path: Some("src/lib.rs".into()),
            status: FileStatus::Modified,
            ..FileMeta::default()
        }
    }

    fn hunks(doc: &diffy_diff::DiffDocument) -> Vec<(u32, u32, u32, u32)> {
        let h = doc.hunks();
        (0..doc.hunk_count() as usize)
            .map(|i| (h.old_start[i], h.old_len[i], h.new_start[i], h.new_len[i]))
            .collect()
    }

    #[test]
    fn semantic_rows_keep_intensities_pairing_and_full_sources() {
        let old = "fn a() {}\nlet x = \"foo\";\nkeep();\n";
        let new = "fn a() {}\nlet x = \"bar\";\nextra();\nkeep();\n";
        let doc = document_from_semantic(
            meta(),
            TextStore::new(old),
            TextStore::new(new),
            &result(vec![vec![
                line(Some(0), Some(0), vec![], vec![]),
                line(
                    Some(1),
                    Some(1),
                    vec![
                        span(8, 9, DftIntensity::UnchangedContext),
                        span(9, 12, DftIntensity::NovelWord),
                        span(12, 13, DftIntensity::UnchangedContext),
                    ],
                    vec![span(9, 12, DftIntensity::NovelWord)],
                ),
                line(None, Some(2), vec![], vec![span(0, 7, DftIntensity::Novel)]),
                line(Some(2), Some(3), vec![], vec![]),
            ]]),
        )
        .unwrap();
        doc.verify_integrity().unwrap();
        assert_eq!(hunks(&doc), [(1, 3, 1, 4)]);
        assert_eq!(
            doc.blocks().kind,
            [BlockKind::Context, BlockKind::Change, BlockKind::Context]
        );
        assert_eq!(doc.stats(0).known(), Some((2, 1)));
        assert!(doc.prefers_structural(0));
        let intensities: Vec<_> = doc
            .semantic_spans(0, Side::Old, 1)
            .iter()
            .map(|s| s.intensity)
            .collect();
        assert_eq!(
            intensities,
            [
                Intensity::UnchangedContext,
                Intensity::NovelWord,
                Intensity::UnchangedContext
            ]
        );
        assert_eq!(
            doc.semantic_spans(0, Side::New, 2)[0].intensity,
            Intensity::Novel
        );
        let rows: Vec<_> = doc.aligned_rows(0).iter().map(|r| (r.old, r.new)).collect();
        assert_eq!(
            rows,
            [
                (Some(0), Some(0)),
                (Some(1), Some(1)),
                (None, Some(2)),
                (Some(2), Some(3))
            ]
        );
        assert_eq!(doc.text(0, Side::New).line_count(), 4);
    }

    #[test]
    fn lines_that_differ_between_chunks_become_changes() {
        let old = "a\nb\nsame\nc\nd\n";
        let new = "A\nb\nsame2\nc\nD\n";
        let doc = document_from_semantic(
            meta(),
            TextStore::new(old),
            TextStore::new(new),
            &result(vec![
                vec![line(
                    Some(0),
                    Some(0),
                    vec![span(0, 1, DftIntensity::Novel)],
                    vec![span(0, 1, DftIntensity::Novel)],
                )],
                vec![line(
                    Some(4),
                    Some(4),
                    vec![span(0, 1, DftIntensity::Novel)],
                    vec![span(0, 1, DftIntensity::Novel)],
                )],
            ]),
        )
        .unwrap();
        doc.verify_integrity().unwrap();
        assert_eq!(hunks(&doc), [(1, 1, 1, 1), (3, 1, 3, 1), (5, 1, 5, 1)]);
        assert_eq!(doc.stats(0).known(), Some((3, 3)));
    }

    #[test]
    fn separate_chunks_stay_separate_hunks_and_bad_spans_are_dropped() {
        let old = "a\n1\n2\n3\n4\nb\n";
        let new = "x\n1\n2\n3\n4\ny\n";
        let doc = document_from_semantic(
            meta(),
            TextStore::new(old),
            TextStore::new(new),
            &result(vec![
                vec![line(
                    Some(0),
                    Some(0),
                    vec![span(0, 9, DftIntensity::Novel)],
                    vec![],
                )],
                vec![
                    line(
                        Some(5),
                        Some(5),
                        vec![],
                        vec![span(3, 2, DftIntensity::Novel)],
                    ),
                    line(Some(6), Some(6), vec![], vec![]),
                ],
            ]),
        )
        .unwrap();
        assert_eq!(hunks(&doc), [(1, 1, 1, 1), (6, 1, 6, 1)]);
        assert_eq!(doc.semantic_spans(0, Side::Old, 0)[0].end, 1);
        assert!(doc.semantic_spans(0, Side::New, 5).is_empty());
    }

    #[test]
    fn added_files_skip_the_semantic_diff() {
        let doc = document_from_changed(&ChangedPath {
            status: FileStatus::Added,
            old_path: None,
            new_path: Some("src/new.rs".to_owned()),
            old_content: Vec::new(),
            new_content: b"fn new() {}\nlet x = 1;\n".to_vec(),
        })
        .unwrap();
        assert_eq!(hunks(&doc), [(0, 0, 1, 2)]);
        assert_eq!(doc.text(0, Side::New).line(1), Some("let x = 1;"));
        assert_eq!(doc.summary(0).status, FileStatus::Added);
    }

    fn repo_spec(left: &str, right: &str) -> CompareSpec {
        CompareSpec {
            mode: CompareMode::TwoDot,
            left_ref: left.to_owned(),
            right_ref: right.to_owned(),
            renderer: RendererKind::Difftastic,
            layout: LayoutMode::Unified,
        }
    }

    #[test]
    fn workdir_compares_read_current_files_and_filter_paths() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        commit_file(&repo, "src/other.rs", "stay\n", "other");
        let head = commit_file(
            &repo,
            "src/lib.rs",
            "fn answer() {\n    old();\n}\n",
            "initial",
        );
        std::fs::write(
            dir.path().join("src/lib.rs"),
            "fn answer() {\n    new();\n}\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/other.rs"), "changed\n").unwrap();
        let mut service = GitService::new();
        service.open(dir.path().to_str().unwrap()).unwrap();
        let output = compare(&repo_spec(&head, WORKDIR_REF), &service, None).unwrap();
        assert_eq!(output.file_count(), 2);
        let lib = output
            .entries
            .iter()
            .position(|entry| entry.path() == "src/lib.rs")
            .unwrap();
        let doc = output.document(lib).unwrap();
        assert_eq!(doc.text(0, Side::Old).line(1), Some("    old();"));
        assert_eq!(doc.text(0, Side::New).line(1), Some("    new();"));
        assert_eq!(doc.stats(0).known(), Some((1, 1)));
        assert!(!doc.semantic_spans(0, Side::New, 1).is_empty());
        let one = compare_path(&repo_spec(&head, WORKDIR_REF), "src/lib.rs", &service).unwrap();
        assert_eq!(one.file_count(), 1);
        assert_eq!(one.entry(0).unwrap().path(), "src/lib.rs");
    }

    #[test]
    fn added_files_in_a_range_are_whole_file_changes() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let base = commit_file(&repo, "README.md", "base\n", "initial");
        let head = commit_file(&repo, "src/new.rs", "fn new() {}\nlet x = 1;\n", "add");
        let mut service = GitService::new();
        service.open(dir.path().to_str().unwrap()).unwrap();
        let output = compare(&repo_spec(&base, &head), &service, None).unwrap();
        let entry = output.entry(0).unwrap();
        assert_eq!(entry.status(), FileStatus::Added);
        assert_eq!(entry.stats.known(), Some((2, 0)));
    }
}
