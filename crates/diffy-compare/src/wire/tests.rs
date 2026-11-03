use std::sync::Arc;

use diffy_diff::{
    AlignedRow, ChangeIntensity, DocumentBuilder, FileMeta, FileStatus, ItemChange, SemanticSpan,
    Side as DocSide, SourceCoverage, TextStore, diff_texts,
};

use super::*;
use crate::{FileDiff, FileInput};

const PATH: &str = "src/lib.rs";

fn input(old: &str, new: &str) -> FileInput {
    FileInput {
        status: FileStatus::Modified,
        old_path: Some(PATH.into()),
        new_path: Some(PATH.into()),
        old: old.as_bytes().to_vec(),
        new: new.as_bytes().to_vec(),
    }
}

fn line_diff(old: &str, new: &str) -> (FileInput, FileDiff) {
    let document = diff_texts(Some(PATH), Some(PATH), Some(old), Some(new), 3);
    let diff = FileDiff {
        document,
        fallback: Some(crate::Fallback::NoGrammar),
    };
    (input(old, new), diff)
}

fn numbered(lines: u32) -> Vec<String> {
    (1..=lines).map(|i| format!("line {i}")).collect()
}

fn text(lines: &[String]) -> String {
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn meta() -> FileMeta {
    FileMeta {
        old_path: Some(Arc::from(PATH)),
        new_path: Some(Arc::from(PATH)),
        status: FileStatus::Modified,
        ..FileMeta::default()
    }
}

/// The text a JavaScript client sees at UTF-16 range `start..end`.
fn utf16_slice(text: &str, start: u32, end: u32) -> String {
    let units: Vec<u16> = text.encode_utf16().collect();
    String::from_utf16_lossy(&units[start as usize..end as usize])
}

fn context(old_line: u32, new_line: u32, text: &str) -> Row {
    Row::Context {
        old_line,
        new_line,
        text: text.into(),
        syntax: Vec::new(),
    }
}

fn span(start: u32, end: u32, intensity: Intensity) -> ChangeSpan {
    ChangeSpan {
        start,
        end,
        intensity,
    }
}

fn modified(row: &Row) -> (&Line, &Line) {
    match row {
        Row::Modified { old, new } => (old, new),
        other => panic!("expected a modified row, got {other:?}"),
    }
}

#[test]
fn line_diff_projects_hunks_gaps_and_word_changes() {
    let old = numbered(30);
    let mut new = old.clone();
    new[14] = "line fifteen".into();
    let (input, diff) = line_diff(&text(&old), &text(&new));
    let file = file(&input, &diff, &WireOptions::default());

    assert_eq!(file.status, Status::Modified);
    assert_eq!(file.language.as_deref(), None, "no_grammar hides the tag");
    assert_eq!((file.additions, file.deletions), (1, 1));
    assert_eq!(file.hunks.len(), 1);
    let hunk = &file.hunks[0];
    assert_eq!(
        (hunk.old_start, hunk.old_len, hunk.new_start, hunk.new_len),
        (12, 7, 12, 7)
    );
    assert_eq!(hunk.header, None);
    assert_eq!(
        hunk.gap_before,
        Some(Gap {
            old_start: 1,
            new_start: 1,
            len: 11
        })
    );
    assert_eq!(
        file.gap_after,
        Some(Gap {
            old_start: 19,
            new_start: 19,
            len: 12
        })
    );
    assert_eq!(hunk.rows.len(), 7);
    assert_eq!(hunk.rows[0], context(12, 12, "line 12"));
    assert_eq!(hunk.rows[6], context(18, 18, "line 18"));
    let (old_line, new_line) = modified(&hunk.rows[3]);
    assert_eq!((old_line.line, old_line.text.as_str()), (15, "line 15"));
    assert_eq!(
        (new_line.line, new_line.text.as_str()),
        (15, "line fifteen")
    );
    assert_eq!(old_line.changes, [span(5, 7, Intensity::NovelWord)]);
    assert_eq!(new_line.changes, [span(5, 12, Intensity::NovelWord)]);
}

#[test]
fn documents_round_trip_through_json_and_omit_empty_fields() {
    let old = numbered(8);
    let mut new = old.clone();
    new[3] = "line four".into();
    new.push("line 9".into());
    let (input, diff) = line_diff(&text(&old), &text(&new));
    let doc = document(
        std::slice::from_ref(&input),
        std::slice::from_ref(&diff),
        &WireOptions::default(),
    );
    assert_eq!(doc.version, FORMAT_VERSION);

    let json = serde_json::to_string(&doc).expect("serializes");
    let back: Document = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back, doc);

    let value: serde_json::Value = serde_json::from_str(&json).expect("parses");
    let file = &value["files"][0];
    assert_eq!(value["version"], 1);
    assert_eq!(file["status"], "modified");
    assert_eq!(
        file["fallback"],
        serde_json::json!({ "kind": "no_grammar", "message": "no grammar for this file" })
    );
    for absent in [
        "binary",
        "language",
        "outline",
        "moves",
        "renames",
        "gap_after",
        "old_missing_newline",
    ] {
        assert!(file.get(absent).is_none(), "{absent} should be omitted");
    }
    let rows = &file["hunks"][0]["rows"];
    assert_eq!(
        rows[0],
        serde_json::json!({ "kind": "context", "old_line": 1, "new_line": 1, "text": "line 1" })
    );
    assert_eq!(rows[3]["kind"], "modified");
    assert_eq!(
        rows[3]["old"]["changes"][0],
        serde_json::json!({ "start": 5, "end": 6, "intensity": "novel_word" })
    );
    assert!(rows[3]["old"].get("syntax").is_none());
    let last = &rows[rows.as_array().expect("rows").len() - 1];
    assert_eq!(
        *last,
        serde_json::json!({ "kind": "added", "new": { "line": 9, "text": "line 9" } })
    );
}

#[test]
fn span_offsets_count_utf16_units() {
    let old = "let s = \"héllo 😀 world\";\n";
    let new = "let s = \"héllo 😀 there\";\n";
    let (input, diff) = line_diff(old, new);
    let file = file(&input, &diff, &WireOptions::default());
    let (old_line, new_line) = modified(&file.hunks[0].rows[0]);
    let [old_span] = old_line.changes[..] else {
        panic!("one old span: {:?}", old_line.changes);
    };
    let [new_span] = new_line.changes[..] else {
        panic!("one new span: {:?}", new_line.changes);
    };
    // `é` is one unit and `😀` two, so "world" starts at 18, not byte 21.
    assert_eq!((old_span.start, old_span.end), (18, 23));
    assert_eq!(
        utf16_slice(&old_line.text, old_span.start, old_span.end),
        "world"
    );
    assert_eq!(
        utf16_slice(&new_line.text, new_span.start, new_span.end),
        "there"
    );
}

#[test]
fn invalid_utf8_lines_show_replacement_characters() {
    let old = b"caf\xe9 au lait\n".to_vec();
    let new = b"caf\xe9 au chocolat\n".to_vec();
    let mut builder = DocumentBuilder::new();
    let mut file_builder = builder.file(
        meta(),
        SourceCoverage::Full,
        TextStore::from_bytes(&old),
        TextStore::from_bytes(&new),
    );
    file_builder.hunk(0, 0).change(1, 1);
    file_builder.finish().expect("builds");
    let diff = FileDiff {
        document: builder.build(),
        fallback: None,
    };
    let input = FileInput {
        old,
        new,
        ..input("", "")
    };
    let projected = file(&input, &diff, &WireOptions::default());
    let (old_line, new_line) = modified(&projected.hunks[0].rows[0]);
    assert_eq!(old_line.text, "caf\u{FFFD} au lait");
    assert_eq!(old_line.changes, [span(8, 12, Intensity::NovelWord)]);
    assert_eq!(
        utf16_slice(
            &new_line.text,
            new_line.changes[0].start,
            new_line.changes[0].end
        ),
        "chocolat"
    );
}

/// A structural document: `render` renamed to `render_preview` with an
/// argument changed after multibyte text, plus an outline, a move, and a
/// rename.
fn semantic_diff() -> (FileInput, FileDiff) {
    let old = "fn keep() {}\nfn render() {\n    draw(\"é😀\", 1);\n}\n";
    let new = "fn keep() {}\nfn render_preview() {\n    draw(\"é😀\", 2);\n}\n";
    let (old_store, new_store) = (TextStore::new(old), TextStore::new(new));
    let at = |store: &TextStore, line: u32, needle: &str| {
        let text = store.display_line(line).expect("line");
        let start = text.find(needle).expect("needle") as u32;
        (start, start + needle.len() as u32)
    };
    let semantic = |(start, end), intensity| SemanticSpan {
        start,
        end,
        intensity,
    };
    let old_spans = [
        semantic(at(&old_store, 1, "render"), ChangeIntensity::Renamed),
        semantic(at(&old_store, 2, "1"), ChangeIntensity::NovelWord),
    ];
    let new_spans = [
        semantic(
            at(&new_store, 1, "render_preview"),
            ChangeIntensity::Renamed,
        ),
        semantic(at(&new_store, 2, "2"), ChangeIntensity::NovelWord),
    ];
    let mut builder = DocumentBuilder::new();
    let mut file_builder = builder.file(meta(), SourceCoverage::Full, old_store, new_store);
    file_builder
        .structural(true)
        .hunk(0, 0)
        .context(1)
        .change(2, 2)
        .context(1);
    for line in 0..4 {
        file_builder.row(AlignedRow {
            old: Some(line),
            new: Some(line),
        });
    }
    for (line, (old_span, new_span)) in (1..).zip(old_spans.iter().zip(&new_spans)) {
        file_builder.spans(DocSide::Old, line, std::slice::from_ref(old_span));
        file_builder.spans(DocSide::New, line, std::slice::from_ref(new_span));
    }
    file_builder
        .outline(vec![diffy_diff::OutlineItem {
            side: DocSide::New,
            line: 1,
            kind: Arc::from("function_item"),
            name: Arc::from("render_preview"),
            change: ItemChange::Edited(2),
        }])
        .renames(vec![diffy_diff::Rename {
            old: Arc::from("render"),
            new: Arc::from("render_preview"),
            uses: 1,
        }])
        .moves(vec![diffy_diff::Move {
            old_line: 0,
            old_lines: 1,
            new_line: 0,
            lines: 1,
            from: None,
            into: Some(Arc::from("render_preview")),
            item: Some(Arc::from("keep")),
        }]);
    file_builder.finish().expect("builds");
    let diff = FileDiff {
        document: builder.build(),
        fallback: None,
    };
    (input(old, new), diff)
}

#[test]
fn semantic_documents_keep_spans_outline_moves_and_renames() {
    let (input, diff) = semantic_diff();
    let file = file(&input, &diff, &WireOptions::default());
    assert_eq!(file.language.as_deref(), Some("rs"));
    assert_eq!(file.fallback, None);
    assert_eq!(
        file.outline,
        [OutlineItem {
            side: Side::New,
            line: 2,
            kind: "function_item".into(),
            name: "render_preview".into(),
            change: ItemChangeKind::Edited,
            edits: Some(2),
        }]
    );
    assert_eq!(
        file.renames,
        [Rename {
            old: "render".into(),
            new: "render_preview".into(),
            uses: 1,
        }]
    );
    assert_eq!(
        file.moves,
        [Move {
            old_start: 1,
            old_len: 1,
            new_start: 1,
            new_len: 1,
            from: None,
            into: Some("render_preview".into()),
            item: Some("keep".into()),
        }]
    );

    let hunk = &file.hunks[0];
    assert_eq!(hunk.header.as_deref(), Some("render_preview"));
    assert_eq!((hunk.gap_before, file.gap_after), (None, None));
    assert_eq!(hunk.rows.len(), 4);
    assert_eq!(hunk.rows[0], context(1, 1, "fn keep() {}"));
    assert_eq!(hunk.rows[3], context(4, 4, "}"));
    let (old_line, new_line) = modified(&hunk.rows[1]);
    assert_eq!(old_line.changes, [span(3, 9, Intensity::Renamed)]);
    assert_eq!(new_line.changes, [span(3, 17, Intensity::Renamed)]);
    let (old_line, new_line) = modified(&hunk.rows[2]);
    // Bytes 19..20, after `é` (2 bytes, 1 unit) and `😀` (4 bytes, 2 units).
    assert_eq!(old_line.changes, [span(16, 17, Intensity::NovelWord)]);
    assert_eq!(new_line.changes, [span(16, 17, Intensity::NovelWord)]);
    assert_eq!(utf16_slice(&new_line.text, 16, 17), "2");

    let json = serde_json::to_string(&file).expect("serializes");
    assert_eq!(
        serde_json::from_str::<File>(&json).expect("deserializes"),
        file
    );
    assert!(json.contains(r#""change":"edited","edits":2"#), "{json}");
}

#[test]
fn hunk_header_needs_an_item_before_the_change() {
    let (input, diff) = semantic_diff();
    let mut document = DocumentBuilder::new();
    let source = diff.document.text(0, DocSide::Old).clone();
    let target = diff.document.text(0, DocSide::New).clone();
    let mut file_builder = document.file(meta(), SourceCoverage::Full, source, target);
    file_builder
        .structural(true)
        .hunk(0, 0)
        .context(1)
        .change(2, 2)
        .context(1)
        .outline(vec![diffy_diff::OutlineItem {
            side: DocSide::Old,
            line: 3,
            kind: Arc::from("function_item"),
            name: Arc::from("later"),
            change: ItemChange::Removed,
        }]);
    file_builder.finish().expect("builds");
    let diff = FileDiff {
        document: document.build(),
        fallback: None,
    };
    let file = file(&input, &diff, &WireOptions::default());
    assert_eq!(file.hunks[0].header, None);
}

#[test]
fn aligned_rows_pair_lines_where_the_semantic_diff_put_them() {
    let old = "keep\none\ntwo\nend\n";
    let new = "keep\none!\ninserted\ntwo!\nend\n";
    let mut builder = DocumentBuilder::new();
    let mut file_builder = builder.file(
        meta(),
        SourceCoverage::Full,
        TextStore::new(old),
        TextStore::new(new),
    );
    file_builder
        .structural(true)
        .hunk(0, 0)
        .context(1)
        .change(2, 3)
        .context(1);
    for (old, new) in [
        (Some(0), Some(0)),
        (Some(1), Some(1)),
        (None, Some(2)),
        (Some(2), Some(3)),
        (Some(3), Some(4)),
    ] {
        file_builder.row(AlignedRow { old, new });
    }
    file_builder.finish().expect("builds");
    let diff = FileDiff {
        document: builder.build(),
        fallback: None,
    };
    let rows = &file(&input(old, new), &diff, &WireOptions::default()).hunks[0].rows;
    let shape: Vec<(&str, Option<u32>, Option<u32>)> = rows
        .iter()
        .map(|row| match row {
            Row::Context {
                old_line, new_line, ..
            } => ("context", Some(*old_line), Some(*new_line)),
            Row::Removed { old } => ("removed", Some(old.line), None),
            Row::Added { new } => ("added", None, Some(new.line)),
            Row::Modified { old, new } => ("modified", Some(old.line), Some(new.line)),
        })
        .collect();
    assert_eq!(
        shape,
        [
            ("context", Some(1), Some(1)),
            ("modified", Some(2), Some(2)),
            ("added", None, Some(3)),
            ("modified", Some(3), Some(4)),
            ("context", Some(4), Some(5)),
        ]
    );
    // Unmarked pairs still get the word diff.
    let (_, new_line) = modified(&rows[1]);
    assert_eq!(new_line.changes, [span(3, 4, Intensity::NovelWord)]);
}

#[test]
fn gaps_track_each_side_and_expand_into_context_rows() {
    let old = numbered(30);
    let mut new = old.clone();
    new.splice(10..10, ["new a".to_string(), "new b".to_string()]);
    let (input, diff) = line_diff(&text(&old), &text(&new));
    let file = file(&input, &diff, &WireOptions::default());
    let hunk = &file.hunks[0];
    assert_eq!(
        (hunk.old_start, hunk.old_len, hunk.new_start, hunk.new_len),
        (8, 6, 8, 8)
    );
    let before = hunk.gap_before.expect("gap before");
    assert_eq!(
        before,
        Gap {
            old_start: 1,
            new_start: 1,
            len: 7
        }
    );
    let after = file.gap_after.expect("gap after");
    assert_eq!(
        after,
        Gap {
            old_start: 14,
            new_start: 16,
            len: 17
        }
    );

    let options = WireOptions::default();
    let rows = expand(&input, &after, &options);
    assert_eq!(rows.len(), 17);
    assert_eq!(rows[0], context(14, 16, "line 14"));
    assert_eq!(rows[16], context(30, 32, "line 30"));

    // The bottom three lines of the gap above the hunk.
    let tail = Gap {
        old_start: 5,
        new_start: 5,
        len: 3,
    };
    let rows = expand(&input, &tail, &options);
    assert_eq!(
        rows,
        [
            context(5, 5, "line 5"),
            context(6, 6, "line 6"),
            context(7, 7, "line 7"),
        ]
    );

    // Past the end of the file: only the lines that exist.
    let past = Gap {
        old_start: 29,
        new_start: 31,
        len: 10,
    };
    assert_eq!(expand(&input, &past, &options).len(), 2);
    // Stale: the sides differ, so nothing is claimed unchanged.
    let stale = Gap {
        old_start: 14,
        new_start: 14,
        len: 3,
    };
    assert!(expand(&input, &stale, &options).is_empty());
    let zero = Gap {
        old_start: 0,
        new_start: 0,
        len: 3,
    };
    assert!(expand(&input, &zero, &options).is_empty());
}

#[test]
fn unchanged_files_are_one_gap_and_added_files_have_none() {
    let lines = text(&numbered(5));
    let (input, diff) = line_diff(&lines, &lines);
    let file = file(&input, &diff, &WireOptions::default());
    assert!(file.hunks.is_empty());
    assert_eq!(
        file.gap_after,
        Some(Gap {
            old_start: 1,
            new_start: 1,
            len: 5
        })
    );

    let document = diff_texts(None, Some(PATH), None, Some("a\nb"), 3);
    let input = FileInput {
        status: FileStatus::Added,
        old_path: None,
        new_path: Some(PATH.into()),
        old: Vec::new(),
        new: b"a\nb".to_vec(),
    };
    let diff = FileDiff {
        document,
        fallback: None,
    };
    let file = super::file(&input, &diff, &WireOptions::default());
    assert_eq!(file.status, Status::Added);
    assert!(file.new_missing_newline);
    assert_eq!(file.gap_after, None);
    let hunk = &file.hunks[0];
    assert_eq!(hunk.gap_before, None);
    assert!(matches!(
        &hunk.rows[..],
        [Row::Added { .. }, Row::Added { .. }]
    ));
}

#[test]
fn binary_files_have_no_rows() {
    let input = FileInput {
        old: b"\0old".to_vec(),
        new: b"\0new".to_vec(),
        ..input("", "")
    };
    let diff = FileDiff {
        document: diff_texts(Some(PATH), Some(PATH), Some(""), Some(""), 3),
        fallback: None,
    };
    let file = file(&input, &diff, &WireOptions::default());
    assert!(file.binary);
    assert!(file.hunks.is_empty() && file.gap_after.is_none());
    let json = serde_json::to_value(&file).expect("serializes");
    assert_eq!(json["binary"], true);
    let gap = Gap {
        old_start: 1,
        new_start: 1,
        len: 1,
    };
    assert!(expand(&input, &gap, &WireOptions::default()).is_empty());
}

#[test]
fn fallbacks_serialize_as_stable_kinds() {
    let cases = [
        (
            crate::Fallback::Semantic(diffy_semantic::Fallback::Timeout),
            serde_json::json!({
                "kind": "semantic",
                "reason": "timeout",
                "message": "exceeded the semantic time budget",
            }),
        ),
        (
            crate::Fallback::Semantic(diffy_semantic::Fallback::TooLarge(5)),
            serde_json::json!({
                "kind": "semantic",
                "reason": "too_large",
                "message": "5 bytes exceeds the semantic byte limit",
            }),
        ),
        (
            crate::Fallback::GrammarUnavailable("zig".into()),
            serde_json::json!({
                "kind": "grammar_unavailable",
                "message": "zig grammar unavailable",
            }),
        ),
        (
            crate::Fallback::RequestBudget,
            serde_json::json!({
                "kind": "request_budget",
                "message": "request time budget exhausted",
            }),
        ),
    ];
    for (fallback, expected) in cases {
        let json = serde_json::to_value(Fallback::from(&fallback)).expect("serializes");
        assert_eq!(json, expected);
    }
}

/// Rust source whose `draw` call changes inside a method.
fn rust_change() -> (FileInput, FileDiff) {
    let old = "fn helper() {}\n\nimpl Parser {\n    fn render_preview(&self) {\n        let s = \"é😀\";\n        draw(1);\n    }\n}\n";
    let new = old.replace("draw(1)", "draw(2)");
    line_diff(old, &new)
}

#[test]
fn syntax_spans_are_classes_sliced_per_line_in_utf16() {
    let Some(store) = diffy_syntax::testing::store_with("rust") else {
        return;
    };
    let (input, diff) = rust_change();
    let options = WireOptions {
        syntax: Some(&store),
        ..WireOptions::default()
    };
    let file = file(&input, &diff, &options);
    let hunk = &file.hunks[0];
    assert_eq!(hunk.header.as_deref(), Some("fn render_preview"));
    let Row::Context { text, syntax, .. } = &hunk.rows[2] else {
        panic!("context row: {:?}", hunk.rows[2]);
    };
    assert_eq!(text, "        let s = \"é😀\";");
    let class_of = |slice: &str| {
        syntax
            .iter()
            .find(|span| utf16_slice(text, span.start, span.end) == slice)
            .map(|span| &*span.class)
    };
    assert_eq!(class_of("let"), Some("keyword"), "{syntax:?}");
    assert_eq!(class_of("\"é😀\""), Some("string"), "{syntax:?}");
    let (old_line, _) = modified(&hunk.rows[3]);
    let draw = old_line.syntax.first().expect("draw is highlighted");
    assert_eq!(utf16_slice(&old_line.text, draw.start, draw.end), "draw");
    assert_eq!(draw.class, "function");

    // Expanded context is highlighted with the whole file's parse.
    let gap = hunk.gap_before.expect("gap before");
    let rows = expand(&input, &gap, &options);
    let Row::Context { syntax, .. } = &rows[0] else {
        panic!("context row: {:?}", rows[0]);
    };
    assert!(syntax.iter().any(|span| span.class == "keyword"));

    let capped = WireOptions {
        syntax: Some(&store),
        syntax_max_bytes: 8,
    };
    let file = super::file(&input, &diff, &capped);
    assert_eq!(file.hunks[0].header, None);
    assert!(file.hunks[0].rows.iter().all(|row| match row {
        Row::Context { syntax, .. } => syntax.is_empty(),
        Row::Removed { old: line } | Row::Added { new: line } => line.syntax.is_empty(),
        Row::Modified { old, new } => old.syntax.is_empty() && new.syntax.is_empty(),
    }));
}

#[test]
fn the_spec_example_matches_the_types() {
    let spec = include_str!("../../FORMAT.md");
    let example = spec
        .split("```json\n")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .expect("FORMAT.md has a JSON example");
    let value: serde_json::Value = serde_json::from_str(example).expect("example parses");
    let document: Document = serde_json::from_value(value.clone()).expect("example is a Document");
    assert_eq!(document.version, FORMAT_VERSION);
    // Nothing in the example is unknown to the types or spelled
    // differently from what they write.
    assert_eq!(serde_json::to_value(&document).expect("serializes"), value);
}

#[test]
fn aligned_rows_that_miss_lines_fall_back_to_positional_pairs() {
    let old = "one\ntwo\n";
    let new = "one!\ntwo!\n";
    let mut builder = DocumentBuilder::new();
    let mut file_builder = builder.file(
        meta(),
        SourceCoverage::Full,
        TextStore::new(old),
        TextStore::new(new),
    );
    file_builder.structural(true).hunk(0, 0).change(2, 2);
    // Only the first pair is aligned; the second line of each side is
    // never mentioned.
    file_builder.row(AlignedRow {
        old: Some(0),
        new: Some(0),
    });
    file_builder.finish().expect("builds");
    let diff = FileDiff {
        document: builder.build(),
        fallback: None,
    };
    let rows = &file(&input(old, new), &diff, &WireOptions::default()).hunks[0].rows;
    assert_eq!(rows.len(), 2, "{rows:?}");
    for (row, number) in rows.iter().zip(1..) {
        let (old_line, new_line) = modified(row);
        assert_eq!((old_line.line, new_line.line), (number, number));
    }
}
