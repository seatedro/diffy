//! One file's chain: trivial cases, then the semantic diff under its
//! budget, else a line diff.

use std::time::{Duration, Instant};

use diffy_diff::{DiffDocument, FileMeta, FileStatus, LineGroupSpan, Side, TextStore};
use diffy_semantic::Limits;
use diffy_syntax::LanguageId;

use crate::document::{Semantics, build, clean_rows, document_from_rows, whole_file};
use crate::grammar::SemanticGrammar;
use crate::line::{binary_document, line_diff};
use crate::{Fallback, FileDiff, FileInput, Options, Result, is_binary, semantic_tag};

/// A file before any diff runs: either already done (binary, added,
/// deleted, unchanged) or two texts to diff.
pub enum Prepared {
    Done(Box<DiffDocument>),
    Text {
        meta: FileMeta,
        old: TextStore,
        new: TextStore,
    },
}

/// Settles files that need no diff.
pub fn prepare(input: &FileInput) -> Result<Prepared> {
    let meta = input.meta();
    if is_binary(&input.old) || is_binary(&input.new) {
        return Ok(Prepared::Done(Box::new(binary_document(meta)?)));
    }
    let old = TextStore::from_bytes(&input.old);
    let new = TextStore::from_bytes(&input.new);
    let done = if input.status == FileStatus::Added || input.old_path.is_none() {
        whole_file(meta, old, new, Side::New)?
    } else if input.status == FileStatus::Deleted || input.new_path.is_none() {
        whole_file(meta, old, new, Side::Old)?
    } else if input.old == input.new {
        build(meta, old, new, |_| {})?
    } else {
        return Ok(Prepared::Text { meta, old, new });
    };
    Ok(Prepared::Done(Box::new(done)))
}

pub(crate) fn diff_one(
    input: &FileInput,
    options: &Options<'_>,
    grammars: &dyn Fn(&LanguageId) -> Option<SemanticGrammar>,
) -> Result<FileDiff> {
    let (meta, old, new) = match prepare(input)? {
        Prepared::Done(document) => {
            return Ok(FileDiff {
                document: *document,
                fallback: None,
            });
        }
        Prepared::Text { meta, old, new } => (meta, old, new),
    };
    let line = |meta: FileMeta, fallback: Fallback| {
        Ok(FileDiff {
            document: line_diff(meta, &input.old, &input.new)?,
            fallback: Some(fallback),
        })
    };
    let Some(tag) = semantic_tag(input.display_path()) else {
        return line(meta, Fallback::NoGrammar);
    };
    if budget(options).is_none() {
        return line(meta, Fallback::RequestBudget);
    }
    let Some(grammar) = grammars(&tag) else {
        return line(meta, Fallback::GrammarUnavailable(tag.to_string()));
    };
    // A first grammar load can be slow; re-check the budget.
    let Some(max_time) = budget(options) else {
        return line(meta, Fallback::RequestBudget);
    };
    let limits = Limits {
        max_time,
        ..Limits::default()
    };
    let embed = |tree: &diffy_semantic::Tree, source: &str| {
        let Some(injector) = &grammar.injector else {
            return Vec::new();
        };
        // Regions repeat a few languages many times; look each up once.
        let mut seen: Vec<(LanguageId, Option<SemanticGrammar>)> = Vec::new();
        let mut out = Vec::new();
        for (language, range) in injector.find(tree, source) {
            let at = match seen.iter().position(|(l, _)| *l == language) {
                Some(at) => at,
                None => {
                    let found = grammars(&language);
                    seen.push((language, found));
                    seen.len() - 1
                }
            };
            if let Some(embedded) = &seen[at].1 {
                out.push(diffy_semantic::Embedded {
                    range,
                    language: embedded.language.clone(),
                    rules: embedded.rules.clone(),
                    name: embedded.name.clone(),
                });
            }
        }
        out
    };
    let diff = diffy_semantic::diff_with(
        &grammar.language,
        &grammar.rules,
        old.as_str(),
        new.as_str(),
        &limits,
        options.vet,
        Some(&embed),
    );
    match diff {
        Ok(diff) => {
            let chunks = clean_rows(diff.chunks, &old, &new);
            let semantics = Semantics {
                outline: diff.outline,
                renames: diff.renames,
                moves: diff.moves,
                groups: diff
                    .groups
                    .into_iter()
                    .map(|g| LineGroupSpan {
                        old: g.old,
                        new: g.new,
                    })
                    .collect(),
            };
            Ok(FileDiff {
                document: document_from_rows(meta, old, new, chunks, true, semantics)?,
                fallback: None,
            })
        }
        Err(fallback) => line(meta, Fallback::Semantic(fallback)),
    }
}

/// The semantic time budget for a file starting now, or `None` once the
/// request's deadline has passed.
fn budget(options: &Options<'_>) -> Option<Duration> {
    let Some(deadline) = options.deadline else {
        return Some(options.file_budget);
    };
    deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
        .map(|left| left.min(options.file_budget))
}
