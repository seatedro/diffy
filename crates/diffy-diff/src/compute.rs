//! Line diffs of two whole texts.

use std::sync::Arc;

use crate::model::{BlockKind, DiffDocument, FileMeta, FileStatus};
use crate::myers::{Budget, Change, diff_with, intern};
use crate::text::TextStore;

/// Context lines around each change in hunks, as `git diff` shows.
pub const DEFAULT_CONTEXT: u32 = 3;

/// Diffs `old` against `new` into a one-file document whose stores hold
/// both whole texts, so its collapsed context can be expanded. `context`
/// lines surround each change; changes closer than twice that share a
/// hunk. A missing side (`None`) makes the file added or deleted.
pub fn diff_texts(
    old_path: Option<&str>,
    new_path: Option<&str>,
    old: Option<&str>,
    new: Option<&str>,
    context: u32,
) -> DiffDocument {
    let status = match (old, new) {
        (None, Some(_)) => FileStatus::Added,
        (Some(_), None) => FileStatus::Deleted,
        _ if old_path.is_some() && new_path.is_some() && old_path != new_path => {
            FileStatus::Renamed
        }
        _ => FileStatus::Modified,
    };
    let old_store = TextStore::new(old.unwrap_or(""));
    let new_store = TextStore::new(new.unwrap_or(""));
    let changes = line_changes(&old_store, &new_store);

    let mut doc = DiffDocument::default();
    doc.begin_file(
        FileMeta {
            old_path: old.and(old_path.or(new_path)).map(Arc::from),
            new_path: new.and(new_path.or(old_path)).map(Arc::from),
            status,
            ..FileMeta::default()
        },
        false,
    );
    let mut blocks = Vec::new();
    for group in hunk_groups(&changes, context) {
        let first = &group[0];
        let last = &group[group.len() - 1];
        // Context is clamped to the file on both sides; equal lines keep
        // the two sides in step, so one clamp serves both.
        let lead = context.min(first.old.start).min(first.new.start);
        let trail = context
            .min(old_store.line_count() - last.old.end)
            .min(new_store.line_count() - last.new.end);
        let (old_from, new_from) = (first.old.start - lead, first.new.start - lead);
        blocks.clear();
        if lead > 0 {
            blocks.push((BlockKind::Context, lead, lead));
        }
        for (i, change) in group.iter().enumerate() {
            if i > 0 {
                let equal = change.old.start - group[i - 1].old.end;
                blocks.push((BlockKind::Context, equal, equal));
            }
            blocks.push((
                BlockKind::Change,
                change.old.len() as u32,
                change.new.len() as u32,
            ));
        }
        if trail > 0 {
            blocks.push((BlockKind::Context, trail, trail));
        }
        let old_len = last.old.end + trail - old_from;
        let new_len = last.new.end + trail - new_from;
        let header = (
            old_from + u32::from(old_len > 0),
            old_len,
            new_from + u32::from(new_len > 0),
            new_len,
        );
        doc.push_hunk(header, Arc::from(""), (old_from, new_from), &blocks);
    }
    doc.finish_file(old_store, new_store);
    doc
}

/// Line changes between two stores. Lines compare with their newline, so
/// a final line gaining or losing one is a change.
pub(crate) fn line_changes(old: &TextStore, new: &TextStore) -> Vec<Change> {
    line_changes_with(old, new, Budget::LINEAR)
}

/// [`line_changes`] under `budget`.
pub(crate) fn line_changes_with(old: &TextStore, new: &TextStore, budget: Budget) -> Vec<Change> {
    fn with_newline(store: &TextStore, i: u32) -> &str {
        let range = store.line_range(i).unwrap_or(0..0);
        let end = (range.end + 1).min(store.as_str().len());
        &store.as_str()[range.start..end]
    }
    let (a, b) = intern(
        (0..old.line_count()).map(|i| with_newline(old, i)),
        (0..new.line_count()).map(|i| with_newline(new, i)),
    );
    diff_with(&a, &b, budget)
}

/// Splits changes into hunks: a change starts a new hunk when more than
/// `2 * context` equal lines separate it from the previous one.
fn hunk_groups(changes: &[Change], context: u32) -> impl Iterator<Item = &[Change]> {
    let mut rest = changes;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let mut end = 1;
        while end < rest.len() && rest[end].old.start - rest[end - 1].old.end <= 2 * context {
            end += 1;
        }
        let (group, tail) = rest.split_at(end);
        rest = tail;
        Some(group)
    })
}
