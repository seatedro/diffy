use std::sync::Arc;

use diffy_diff::{
    Comparison, ComparisonOptions, ContextPolicy, DiffDocument, Expansion, FileDiffSnapshot,
    FileStatus, Mode, Projection, Side,
};

use super::DiffViewState;
use super::prepared::FileFact;
use super::state::Segment;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preparation {
    pub mode: Mode,
    pub policy: ContextPolicy,
    pub comparison: ComparisonOptions,
}

#[derive(Debug, Clone)]
pub struct PreparedSegment {
    snapshot: FileDiffSnapshot,
    preparation: Preparation,
    expansion: Expansion,
    comparison: Comparison,
    projection: Projection,
    alternate: Projection,
}

impl PreparedSegment {
    pub fn prepare(snapshot: FileDiffSnapshot, preparation: Preparation) -> Self {
        let doc = &snapshot.diff;
        let expansion = Expansion::with_policy(doc, preparation.policy);
        let comparison = Comparison::new(doc, preparation.comparison);
        let project = |mode| Projection::with_comparison(doc, mode, &expansion, &comparison);
        let projection = project(preparation.mode);
        let alternate = project(other(preparation.mode));
        Self {
            snapshot,
            preparation,
            expansion,
            comparison,
            projection,
            alternate,
        }
    }

    pub fn snapshot(&self) -> &FileDiffSnapshot {
        &self.snapshot
    }

    pub fn preparation(&self) -> Preparation {
        self.preparation
    }
}

fn other(mode: Mode) -> Mode {
    match mode {
        Mode::Unified => Mode::Split,
        Mode::Split => Mode::Unified,
    }
}

pub(super) fn segment_for(
    view: &DiffViewState,
    doc: &Arc<DiffDocument>,
    slot: u32,
    prepared: Option<Arc<PreparedSegment>>,
) -> Segment {
    let wanted = Preparation {
        mode: view.mode,
        policy: view.context_policy,
        comparison: view.comparison,
    };
    let fresh = || {
        Segment::new(
            doc.clone(),
            slot,
            view.generations,
            view.mode,
            view.context_policy,
            view.comparison,
        )
    };
    let Some(prepared) = prepared else {
        return fresh();
    };
    let reusable = Arc::ptr_eq(&prepared.snapshot.diff, doc)
        && prepared.preparation.policy == wanted.policy
        && prepared.preparation.comparison == wanted.comparison;
    if !reusable {
        return fresh();
    }
    let prepared = Arc::unwrap_or_clone(prepared);
    let (projection, alternate) = if prepared.preparation.mode == wanted.mode {
        (
            prepared.projection,
            (other(wanted.mode), prepared.alternate),
        )
    } else {
        (
            prepared.alternate,
            (prepared.preparation.mode, prepared.projection),
        )
    };
    Segment::from_parts(
        doc.clone(),
        slot,
        view.generations,
        prepared.expansion,
        prepared.comparison,
        projection,
        Some(alternate),
    )
}

/// Facts about `file` its line rows do not show. See [`FileFact`]. A
/// missing final newline is a fact only where it differs between the sides
/// (or the file was added or deleted), since an unchanged one is not part
/// of the change.
pub(crate) fn file_facts(doc: &DiffDocument, file: u32) -> Vec<FileFact> {
    let facts = doc.facts(file);
    let mut out = Vec::new();
    if facts.binary {
        out.push(FileFact::Binary);
    }
    if let Some((old, new)) = facts.mode_change {
        out.push(FileFact::ModeChange { old, new });
    }
    match facts.status {
        FileStatus::Renamed if !facts.has_hunks && !facts.binary => {
            out.push(FileFact::RenameOnly);
        }
        FileStatus::Copied if !facts.has_hunks && !facts.binary => out.push(FileFact::CopyOnly),
        _ => {}
    }
    let eof = [facts.old_missing_newline, facts.new_missing_newline];
    for side in [Side::Old, Side::New] {
        let differs = match facts.status {
            FileStatus::Added => side == Side::New,
            FileStatus::Deleted => side == Side::Old,
            _ => eof[0] != eof[1],
        };
        if eof[side as usize] && differs {
            out.push(FileFact::NoNewlineAtEof(side));
        }
    }
    out
}
