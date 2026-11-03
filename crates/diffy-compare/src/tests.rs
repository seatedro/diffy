use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use diffy_diff::{
    BlockKind, ChangeIntensity as Intensity, DiffDocument, FileMeta, FileStatus, SemanticSpan,
    Side, TextStore,
};
use diffy_semantic::Row;

use crate::document::{Semantics, clean_rows, document_from_rows};
use crate::{
    Error, Fallback, FileInput, GrammarStore, Observer, Options, diff_file, diff_files, line_diff,
};

fn span(start: u32, end: u32, intensity: Intensity) -> SemanticSpan {
    SemanticSpan {
        start,
        end,
        intensity,
    }
}

fn line(
    old: Option<u32>,
    new: Option<u32>,
    old_spans: Vec<SemanticSpan>,
    new_spans: Vec<SemanticSpan>,
) -> Row {
    Row {
        old,
        new,
        spans: [old_spans, new_spans],
    }
}

fn rows_document(old: &str, new: &str, chunks: Vec<Vec<Row>>) -> DiffDocument {
    let (old, new) = (TextStore::new(old), TextStore::new(new));
    let chunks = clean_rows(chunks, &old, &new);
    document_from_rows(meta(), old, new, chunks, true, Semantics::default()).unwrap()
}

fn meta() -> FileMeta {
    FileMeta {
        old_path: Some("src/lib.rs".into()),
        new_path: Some("src/lib.rs".into()),
        status: FileStatus::Modified,
        ..FileMeta::default()
    }
}

fn hunks(doc: &DiffDocument) -> Vec<(u32, u32, u32, u32)> {
    let h = doc.hunks();
    (0..doc.hunk_count() as usize)
        .map(|i| (h.old_start[i], h.old_len[i], h.new_start[i], h.new_len[i]))
        .collect()
}

fn modified(path: &str, old: &str, new: &str) -> FileInput {
    FileInput {
        status: FileStatus::Modified,
        old_path: Some(path.to_owned()),
        new_path: Some(path.to_owned()),
        old: old.as_bytes().to_vec(),
        new: new.as_bytes().to_vec(),
    }
}

const OLD_RS: &str = "fn answer() {\n    old();\n}\n";
const NEW_RS: &str = "fn answer() {\n    new();\n}\n";

#[test]
fn semantic_rows_keep_intensities_pairing_and_full_sources() {
    let old = "fn a() {}\nlet x = \"foo\";\nkeep();\n";
    let new = "fn a() {}\nlet x = \"bar\";\nextra();\nkeep();\n";
    let doc = rows_document(
        old,
        new,
        vec![vec![
            line(Some(0), Some(0), vec![], vec![]),
            line(
                Some(1),
                Some(1),
                vec![
                    span(8, 9, Intensity::UnchangedContext),
                    span(9, 12, Intensity::NovelWord),
                    span(12, 13, Intensity::UnchangedContext),
                ],
                vec![span(9, 12, Intensity::NovelWord)],
            ),
            line(None, Some(2), vec![], vec![span(0, 7, Intensity::Novel)]),
            line(Some(2), Some(3), vec![], vec![]),
        ]],
    );
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
    let doc = rows_document(
        old,
        new,
        vec![
            vec![line(
                Some(0),
                Some(0),
                vec![span(0, 1, Intensity::Novel)],
                vec![span(0, 1, Intensity::Novel)],
            )],
            vec![line(
                Some(4),
                Some(4),
                vec![span(0, 1, Intensity::Novel)],
                vec![span(0, 1, Intensity::Novel)],
            )],
        ],
    );
    doc.verify_integrity().unwrap();
    assert_eq!(hunks(&doc), [(1, 1, 1, 1), (3, 1, 3, 1), (5, 1, 5, 1)]);
    assert_eq!(doc.stats(0).known(), Some((3, 3)));
}

#[test]
fn separate_chunks_stay_separate_hunks_and_bad_spans_are_dropped() {
    let old = "a\n1\n2\n3\n4\nb\n";
    let new = "x\n1\n2\n3\n4\ny\n";
    let doc = rows_document(
        old,
        new,
        vec![
            vec![line(
                Some(0),
                Some(0),
                vec![span(0, 9, Intensity::Novel)],
                vec![],
            )],
            vec![
                line(Some(5), Some(5), vec![], vec![span(3, 2, Intensity::Novel)]),
                line(Some(6), Some(6), vec![], vec![]),
            ],
        ],
    );
    assert_eq!(hunks(&doc), [(1, 1, 1, 1), (6, 1, 6, 1)]);
    assert_eq!(doc.semantic_spans(0, Side::Old, 0)[0].end, 1);
    assert!(doc.semantic_spans(0, Side::New, 5).is_empty());
}

#[test]
fn added_files_skip_the_semantic_diff() {
    let store = GrammarStore::none();
    let diff = diff_file(
        &FileInput {
            status: FileStatus::Added,
            old_path: None,
            new_path: Some("src/new.rs".to_owned()),
            old: Vec::new(),
            new: b"fn new() {}\nlet x = 1;\n".to_vec(),
        },
        &Options::new(&store),
    )
    .unwrap();
    let doc = diff.document;
    assert_eq!(diff.fallback, None);
    assert_eq!(hunks(&doc), [(0, 0, 1, 2)]);
    assert_eq!(doc.text(0, Side::New).line(1), Some("let x = 1;"));
    assert_eq!(doc.summary(0).status, FileStatus::Added);
}

#[test]
fn files_without_a_grammar_get_line_diffs() {
    let store = GrammarStore::none();
    let input = modified("src/lib.rs", "a\nb\n", "a\nc\n");
    let diff = diff_file(&input, &Options::new(&store)).unwrap();
    let fallback = diff.fallback.unwrap();
    assert!(matches!(fallback, Fallback::GrammarUnavailable(_)));
    assert!(fallback.to_string().ends_with(" grammar unavailable"));
    assert_eq!(hunks(&diff.document), [(1, 2, 1, 2)]);
}

#[test]
fn rust_files_get_a_semantic_diff_within_the_budget() {
    let Some(store) = diffy_syntax::testing::store_with("rust") else {
        return;
    };
    let mut options = Options::new(&store);
    options.deadline = Some(Instant::now() + Duration::from_secs(60));
    let diff = diff_file(&modified("src/lib.rs", OLD_RS, NEW_RS), &options).unwrap();
    assert_eq!(diff.fallback, None);
    assert!(diff.document.prefers_structural(0));
    assert!(!diff.document.semantic_spans(0, Side::New, 1).is_empty());
}

#[test]
fn files_after_the_deadline_get_line_diffs() {
    // No grammar is needed: the deadline is checked before one loads.
    let store = GrammarStore::none();
    let mut options = Options::new(&store);
    options.deadline = Some(Instant::now());
    let inputs = [
        modified("src/lib.rs", OLD_RS, NEW_RS),
        FileInput {
            status: FileStatus::Added,
            old_path: None,
            new_path: Some("src/new.rs".to_owned()),
            new: b"fn new() {}\n".to_vec(),
            ..FileInput::default()
        },
        modified("src/main.rs", OLD_RS, NEW_RS),
    ];
    let diffs = diff_files(&inputs, &options).unwrap();
    let fallbacks: Vec<_> = diffs.iter().map(|diff| diff.fallback.clone()).collect();
    assert_eq!(
        fallbacks,
        [
            Some(Fallback::RequestBudget),
            None,
            Some(Fallback::RequestBudget)
        ]
    );
    let line = line_diff(inputs[0].meta(), OLD_RS.as_bytes(), NEW_RS.as_bytes()).unwrap();
    assert_eq!(hunks(&diffs[0].document), hunks(&line));
    assert!(!diffs[0].document.prefers_structural(0));
}

#[test]
fn cancelled_requests_stop() {
    let store = GrammarStore::none();
    let cancel = AtomicBool::new(true);
    let mut options = Options::new(&store);
    options.cancel = Some(&cancel);
    let input = modified("src/lib.rs", OLD_RS, NEW_RS);
    assert_eq!(diff_file(&input, &options).unwrap_err(), Error::Cancelled);
    assert_eq!(
        diff_files(&[input.clone(), input], &options).unwrap_err(),
        Error::Cancelled
    );
}

struct CancelAfterFirst<'a> {
    cancel: &'a AtomicBool,
    files: AtomicU32,
}

impl Observer for CancelAfterFirst<'_> {
    fn file(&self, _path: &str, work: &mut dyn FnMut()) {
        self.files.fetch_add(1, Ordering::Relaxed);
        work();
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[test]
fn cancelling_mid_request_stops_picking_up_files() {
    let store = GrammarStore::none();
    let cancel = AtomicBool::new(false);
    let observer = CancelAfterFirst {
        cancel: &cancel,
        files: AtomicU32::new(0),
    };
    let mut options = Options::new(&store);
    options.cancel = Some(&cancel);
    options.workers = 1;
    options.observer = Some(&observer);
    let inputs = vec![modified("a.txt", "a\n", "b\n"); 8];
    assert_eq!(diff_files(&inputs, &options).unwrap_err(), Error::Cancelled);
    assert_eq!(observer.files.load(Ordering::Relaxed), 1);
}
