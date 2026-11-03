//! Line diffs: what a file gets when the semantic diff does not run.

use diffy_diff::{
    DiffDocument, DocumentBuilder, FileMeta, SourceCoverage, TextStore, diff_from_changes,
    myers::Change,
};
use imara_diff::{Algorithm, Diff, InternedInput};

use crate::{Error, Result, is_binary};

/// Unchanged lines shown around each change in a line diff.
pub const CONTEXT_LINES: u32 = 3;

/// A line diff of `old` and `new` as one file described by `meta`, or a
/// binary placeholder when either side looks binary.
pub fn line_diff(mut meta: FileMeta, old: &[u8], new: &[u8]) -> Result<DiffDocument> {
    if is_binary(old) || is_binary(new) {
        return binary_document(meta);
    }
    let changes = line_changes(old, new);
    let (old, new) = (TextStore::from_bytes(old), TextStore::from_bytes(new));
    meta.binary = false;
    diff_from_changes(meta, old, new, &changes, CONTEXT_LINES)
        .map_err(|error| Error::Build(error.to_string()))
}

/// A document for a binary file: its metadata and no text.
pub fn binary_document(mut meta: FileMeta) -> Result<DiffDocument> {
    meta.binary = true;
    let mut builder = DocumentBuilder::new();
    builder
        .file(
            meta,
            SourceCoverage::PatchOnly,
            TextStore::default(),
            TextStore::default(),
        )
        .finish()
        .map_err(|error| Error::Build(error.to_string()))?;
    Ok(builder.build())
}

/// Histogram line diff with Git's slider heuristics, as `git diff` and
/// gix compute it.
pub(crate) fn line_changes(old: &[u8], new: &[u8]) -> Vec<Change> {
    let input = InternedInput::new(old, new);
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_lines(&input);
    diff.hunks()
        .map(|hunk| Change {
            old: hunk.before,
            new: hunk.after,
        })
        .collect()
}
