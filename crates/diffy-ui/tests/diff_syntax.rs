//! Syntax colors of diffs, through the diff view's syntax bridge, with real
//! grammar packs.
//!
//! Packs come from `diffy_syntax::testing::pack_root` (build them with
//! `cargo run -p syntax-pack -- build rust javascript html python`); a test
//! whose packs are missing skips unless `DIFFY_REQUIRE_SYNTAX_PACKS=1`.
//! Run with `cargo test -p diffy-ui --test diff_syntax`.

use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use std::ops::Range;

use diffy_diff::{DiffDocument, FileSources, Side, TextStore, diff_texts, parse_unified};
use diffy_syntax::{
    GrammarStore, HighlightKind, HighlightPart, HighlightSpan, HighlightWorker, Highlighted,
    testing,
};
use diffy_ui::diff_view::syntax::{DiffSyntax, SyntaxBudget, SyntaxStatus};

const COLORS: &[HighlightKind] = &[
    HighlightKind::Keyword,
    HighlightKind::String,
    HighlightKind::Comment,
    HighlightKind::Function,
];

/// `kind:text` for each run of the first line of `side` containing
/// `needle` whose kind is in [`COLORS`], in order.
fn dump(syntax: &DiffSyntax, doc: &DiffDocument, side: Side, needle: &str) -> String {
    let store = doc.text(0, side);
    let line = (0..store.line_count())
        .find(|&i| store.display_line(i).unwrap().contains(needle))
        .unwrap_or_else(|| panic!("no {side:?} line contains {needle:?}"));
    let text = store.display_line(line).unwrap();
    let (spans, tones) = syntax.spans(0, side, store.display_range(line).unwrap());
    spans
        .iter()
        .zip(tones.iter())
        .filter(|(_, kind)| COLORS.contains(kind))
        .map(|(span, kind)| format!("{}:{}", kind.name(), &text[span.range.clone()]))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A bridge over `store` that has every first result for `doc`.
fn highlighted(doc: &DiffDocument, store: GrammarStore) -> DiffSyntax {
    let mut syntax = DiffSyntax::default();
    syntax.enable(store);
    syntax.request(doc, 1);
    syntax.finish_pending();
    syntax
}

/// A wake callback that signals the returned receiver.
fn woken(syntax: &mut DiffSyntax) -> Receiver<()> {
    let (tx, rx) = channel();
    syntax.set_wake(move || {
        let _ = tx.send(());
    });
    rx
}

fn wait(woke: &Receiver<()>) {
    woke.recv_timeout(Duration::from_secs(60))
        .expect("the worker publishes a result");
}

/// `count` numbered filler lines, so hunks stay apart.
fn filler(count: usize) -> String {
    (0..count).map(|i| format!("x{i}\n")).collect()
}

// Catches whole files being highlighted hunk by hunk, which loses lexical
// state between hunks: a template string spanning the gap between two
// hunks must color the changed line of the second.
#[test]
fn template_string_spanning_two_hunks_stays_a_string() {
    let Some(store) = testing::store_with("javascript") else {
        return;
    };
    let old = format!("const t = `\nalpha\n{}beta\n`;\n", filler(10));
    let new = format!("const t = `\nalpha 2\n{}beta 2\n`;\n", filler(10));
    let doc = diff_texts(Some("a.js"), Some("a.js"), Some(&old), Some(&new), 3);
    assert_eq!(
        doc.hunk_count(),
        2,
        "the gap between hunks is in the string"
    );
    let syntax = highlighted(&doc, store);

    assert_eq!(dump(&syntax, &doc, Side::New, "beta 2"), "string:beta 2");
}

// Catches a patch's hunks being highlighted as one concatenated source: a
// comment opened in one hunk (and closed in a later one) must not color
// the hunk between them, whose real surroundings the patch does not show.
#[test]
fn patch_hunks_do_not_share_lexical_state() {
    let Some(store) = testing::store_with("rust") else {
        return;
    };
    let patch = "\
--- a/lib.rs
+++ b/lib.rs
@@ -1,2 +1,2 @@
 /* opened
-let a = 1;
+let a = 2;
@@ -20,2 +20,2 @@
 x
-let b = 1;
+let b = 2;
@@ -40,2 +40,2 @@
 */
-y
+z
";
    let doc = parse_unified(patch).unwrap();
    let syntax = highlighted(&doc, store);

    assert_eq!(dump(&syntax, &doc, Side::New, "let b"), "keyword:let");
}

// Catches a hydrated patch keeping its hunk-by-hunk colors: once the
// exact sources arrive, a hunk inside a block comment opened above it is
// recolored as comment and the file reports whole-source colors.
#[test]
fn hydrated_patch_recolors_from_whole_sources() {
    let Some(store) = testing::store_with("rust") else {
        return;
    };
    let old = format!("/* notes\n{}let x = 1;\n*/\n", filler(8));
    let new = format!("/* notes\n{}let x = 2;\n*/\n", filler(8));
    let patch = diffy_diff::write_unified(&diff_texts(
        Some("lib.rs"),
        Some("lib.rs"),
        Some(&old),
        Some(&new),
        3,
    ));
    let doc = parse_unified(&patch).unwrap();
    let mut syntax = highlighted(&doc, store);
    let before = (
        syntax.file_status(0),
        dump(&syntax, &doc, Side::New, "x = 2"),
    );

    let sources = FileSources {
        old: Some(TextStore::new(old)),
        new: Some(TextStore::new(new)),
    };
    let hydrated = doc.hydrate_file(0, sources).unwrap();
    syntax.request(&hydrated, 2);
    syntax.finish_pending();

    assert_eq!(
        [
            before,
            (
                syntax.file_status(0),
                dump(&syntax, &hydrated, Side::New, "x = 2")
            )
        ],
        [
            (Some(SyntaxStatus::PartialSource), "keyword:let".to_owned()),
            (Some(SyntaxStatus::Ready), "comment:let x = 2;".to_owned()),
        ]
    );
}

// Catches injected languages losing their host's lexical state across a
// hunk boundary: a script element opened before the hunk makes its
// changed line JavaScript.
#[test]
fn script_opened_before_the_hunk_takes_javascript_colors() {
    let Some(store) = testing::store_with_all(&["html", "javascript"]) else {
        return;
    };
    let old = format!("<script>\n{}const a = f(1);\n</script>\n", filler(8));
    let new = format!("<script>\n{}const a = f(2);\n</script>\n", filler(8));
    let doc = diff_texts(Some("a.html"), Some("a.html"), Some(&old), Some(&new), 3);
    let syntax = highlighted(&doc, store);

    assert_eq!(
        dump(&syntax, &doc, Side::New, "f(2)"),
        "keyword:const function:f"
    );
}

// Catches both sides of a rename taking the new path's language: the old
// side of a Python file renamed to Rust is still Python.
#[test]
fn renamed_sides_take_their_own_languages() {
    let Some(store) = testing::store_with_all(&["python", "rust"]) else {
        return;
    };
    let patch = "\
diff --git a/tool.py b/tool.rs
similarity index 10%
rename from tool.py
rename to tool.rs
--- a/tool.py
+++ b/tool.rs
@@ -1 +1 @@
-def run(): pass
+fn run() {}
";
    let doc = parse_unified(patch).unwrap();
    let syntax = highlighted(&doc, store);

    assert_eq!(
        [
            dump(&syntax, &doc, Side::Old, "run"),
            dump(&syntax, &doc, Side::New, "run"),
        ],
        [
            "keyword:def function:run keyword:pass",
            "keyword:fn function:run"
        ]
    );
}

// Catches a grammar that arrives after the first highlight never reaching
// the diff, or the side still claiming to wait for it: the side renders
// plain while pending, then the worker's newer revision recolors it and
// wakes the app to take it.
#[test]
fn late_grammar_recolors_the_waiting_side() {
    let Some(store) = testing::store_deferring(&["rust"], &["rust"]) else {
        return;
    };
    let doc = diff_texts(
        Some("lib.rs"),
        Some("lib.rs"),
        Some("fn a() {}\n"),
        Some("fn b() {}\n"),
        3,
    );
    let mut syntax = DiffSyntax::default();
    let woke = woken(&mut syntax);
    syntax.enable(store.clone());
    syntax.request(&doc, 1);
    syntax.finish_pending();
    let before = (
        syntax.status(0, Side::New),
        dump(&syntax, &doc, Side::New, "fn b"),
    );

    store.release_deferred();
    // Both sides recolor. The worker wakes after handing over each result,
    // so wakes for the first results can still arrive after
    // `finish_pending` took them: take results until no side waits.
    while [Side::Old, Side::New]
        .into_iter()
        .any(|side| syntax.status(0, side) == Some(SyntaxStatus::Pending))
    {
        wait(&woke);
        syntax.poll(1);
    }

    assert_eq!(
        [
            before,
            (
                syntax.status(0, Side::New),
                dump(&syntax, &doc, Side::New, "fn b")
            )
        ],
        [
            (Some(SyntaxStatus::Pending), String::new()),
            (
                Some(SyntaxStatus::Ready),
                "keyword:fn function:b".to_owned()
            ),
        ]
    );
}

// Catches a result for replaced text landing on its replacement: the
// highlight of the first document is still in flight when the file is
// replaced by text too large to highlight, and must not color it.
#[test]
fn result_for_replaced_text_is_dropped() {
    let Some(store) = testing::store_with("rust") else {
        return;
    };
    let first = diff_texts(
        Some("lib.rs"),
        Some("lib.rs"),
        Some("fn a() {}\n"),
        Some("fn b() {}\n"),
        3,
    );
    let large = format!("let long = 1; {}\n", "/".repeat(80));
    let second = diff_texts(
        Some("lib.rs"),
        Some("lib.rs"),
        Some("fn a() {}\n"),
        Some(&large),
        3,
    );
    let mut syntax = DiffSyntax::default();
    let woke = woken(&mut syntax);
    syntax.enable(store);
    syntax.request(&first, 1);
    // Both sides' results are waiting, not yet taken.
    wait(&woke);
    wait(&woke);

    syntax.set_budget(SyntaxBudget { side_bytes: 64 });
    syntax.request(&second, 2);
    syntax.poll(2);

    assert_eq!(
        (
            syntax.status(0, Side::New),
            dump(&syntax, &second, Side::New, "long")
        ),
        (Some(SyntaxStatus::Limited), String::new())
    );
}

// Catches a status that misstates how far colors can be trusted: whole
// files are ready, patch hunks best effort, languages without a grammar
// and oversized sides plain, each named so the header can say so.
#[test]
fn file_status_names_how_far_colors_can_be_trusted() {
    let Some(store) = testing::store_with("rust") else {
        return;
    };
    let texts = |path: &str, new: &str| {
        diff_texts(Some(path), Some(path), Some("fn a() {}\n"), Some(new), 3)
    };
    let patch = "--- a/lib.rs\n+++ b/lib.rs\n@@ -1 +1 @@\n-fn a() {}\n+fn b() {}\n";
    let cases = [
        (
            "whole files",
            texts("lib.rs", "fn b() {}\n"),
            SyntaxStatus::Ready,
        ),
        (
            "patch",
            parse_unified(patch).unwrap(),
            SyntaxStatus::PartialSource,
        ),
        (
            "no pack",
            texts("lib.klingon", "fn b() {}\n"),
            SyntaxStatus::MissingGrammar,
        ),
        (
            "no extension",
            texts("Makefile", "fn b() {}\n"),
            SyntaxStatus::MissingGrammar,
        ),
        (
            "too large",
            texts("lib.rs", &"// long\n".repeat(20)),
            SyntaxStatus::Limited,
        ),
    ];
    for (name, doc, expected) in cases {
        let mut syntax = DiffSyntax::default();
        syntax.set_budget(SyntaxBudget { side_bytes: 100 });
        syntax.enable(store.clone());
        syntax.request(&doc, 1);
        syntax.finish_pending();

        assert_eq!(syntax.file_status(0), Some(expected), "{name}");
    }
}

// Catches streamed parts replacing each other (only the last window
// would keep colors), the focus's inexact colors outliving the exact
// ones, or the status claiming Ready mid-stream: exact parts add up in
// order, the inexact window colors only past them, and the progress
// counts exact bytes.
#[test]
fn streamed_parts_add_up_and_the_focus_colors_ahead() {
    let Some(store) = testing::store_with("rust") else {
        return;
    };
    let doc = diff_texts(
        Some("a.rs"),
        Some("a.rs"),
        Some("x\n"),
        Some("aaaa\nbbbb\ncccc\ndddd\n"),
        3,
    );
    let mut syntax = DiffSyntax::default();
    syntax.enable_shared(&HighlightWorker::gone(), store);
    syntax.request(&doc, 1);
    let source = doc.text(0, Side::New).shared().clone();
    let mut revision = 0;
    let mut feed =
        |syntax: &mut DiffSyntax, range: Range<u32>, exact, span: (u32, HighlightKind)| {
            syntax.take(
                Highlighted {
                    slot: 1,
                    generation: 2,
                    revision,
                    source: source.clone(),
                    spans: vec![HighlightSpan {
                        offset: span.0,
                        length: 4,
                        kind: span.1,
                    }],
                    pending: false,
                    unresolved: Vec::new(),
                    part: Some(HighlightPart { range, exact }),
                },
                1,
            );
            revision += 1;
        };
    let dump = |syntax: &DiffSyntax| {
        let (spans, tones) = syntax.spans(0, Side::New, 0..20);
        let colored: Vec<String> = spans
            .iter()
            .zip(tones.iter())
            .filter(|(_, kind)| **kind != HighlightKind::Normal)
            .map(|(s, kind)| format!("{}:{:?}", kind.name(), s.range))
            .collect();
        (colored.join(" "), syntax.status(0, Side::New))
    };

    feed(&mut syntax, 10..20, false, (10, HighlightKind::Keyword));
    feed(&mut syntax, 0..5, true, (0, HighlightKind::String));
    let midway = dump(&syntax);
    feed(&mut syntax, 5..20, true, (15, HighlightKind::Comment));
    let done = dump(&syntax);

    assert_eq!(
        midway,
        (
            "string:0..4 keyword:10..14".to_owned(),
            Some(SyntaxStatus::Streaming { permille: 250 })
        )
    );
    assert_eq!(
        done,
        (
            "string:0..4 comment:15..19".to_owned(),
            Some(SyntaxStatus::Ready)
        )
    );
}
