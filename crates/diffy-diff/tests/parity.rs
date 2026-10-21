use diffy_diff::{
    AlignedRow, BuildError, ChangeIntensity, Comparison, ComparisonOptions, ContentChange,
    DiffDocument, DiffStats, DocumentBuilder, Expansion, FileMeta, FileStatus, Mode, ObjectId,
    PairingMode, Projection, RowKind, SemanticError, SemanticSpan, Side, SourceCoverage, TextStore,
    diff_from_changes, myers::Change, parse_unified, write_unified,
};

fn fixture(name: &str) -> DiffDocument {
    let text = match name {
        "binary_mode" => include_str!("../fixtures/patches/binary_mode.patch"),
        "long_line" => include_str!("../fixtures/patches/long_line.patch"),
        "many_small_files" => include_str!("../fixtures/patches/many_small_files.patch"),
        "mode_only" => include_str!("../fixtures/patches/mode_only.patch"),
        "no_newline" => include_str!("../fixtures/patches/no_newline.patch"),
        "rename_mode" => include_str!("../fixtures/patches/rename_mode.patch"),
        _ => unreachable!(),
    };
    let doc = parse_unified(text).unwrap();
    doc.verify_integrity().unwrap();
    doc
}

fn hunks(doc: &DiffDocument, file: u32) -> Vec<(u32, u32, u32, u32)> {
    let h = doc.hunks();
    doc.files().hunks[file as usize]
        .clone()
        .map(|i| {
            let i = i as usize;
            (h.old_start[i], h.old_len[i], h.new_start[i], h.new_len[i])
        })
        .collect()
}

fn ids(doc: &DiffDocument, file: u32) -> (Option<&str>, Option<&str>) {
    let meta = &doc.files().meta[file as usize];
    (
        meta.old_id.as_ref().map(ObjectId::as_str),
        meta.new_id.as_ref().map(ObjectId::as_str),
    )
}

#[test]
fn many_small_files_keep_paths_ids_and_hunk_coordinates() {
    let doc = fixture("many_small_files");
    let rows: Vec<_> = doc
        .summaries()
        .map(|s| (s.path.to_string(), s.status, s.additions, s.deletions))
        .collect();
    assert_eq!(
        rows,
        [
            ("src/a.rs".to_owned(), FileStatus::Modified, 1, 1),
            ("src/b.rs".to_owned(), FileStatus::Modified, 1, 1),
            ("src/c.rs".to_owned(), FileStatus::Modified, 1, 0),
        ]
    );
    assert_eq!(hunks(&doc, 0), [(1, 3, 1, 3)]);
    assert_eq!(hunks(&doc, 1), [(1, 4, 1, 4)]);
    assert_eq!(hunks(&doc, 2), [(1, 3, 1, 4)]);
    assert_eq!(ids(&doc, 1), (Some("3333333"), Some("4444444")));
    assert_eq!(
        doc.stats(2),
        DiffStats::Known {
            additions: 1,
            deletions: 0
        }
    );
    let meta = &doc.files().meta[0];
    assert_eq!(meta.old_mode.as_deref(), Some("100644"));
    assert_eq!(meta.new_mode.as_deref(), Some("100644"));
    assert_eq!(doc.facts(0).mode_change, None);
    assert_eq!(doc.summary(0).label(), "modified");
}

#[test]
fn rename_with_mode_and_content_change_reports_every_fact() {
    let doc = fixture("rename_mode");
    let summary = doc.summaries().next().unwrap();
    assert_eq!(summary.path.as_ref(), "src/new_name.rs");
    assert_eq!(summary.old_path.as_deref(), Some("src/old_name.rs"));
    let facts = doc.facts(0);
    assert_eq!(facts.status, FileStatus::Renamed);
    assert_eq!(
        facts
            .mode_change
            .as_ref()
            .map(|(o, n)| (o.as_ref(), n.as_ref())),
        Some(("100644", "100755"))
    );
    assert_eq!(facts.content, ContentChange::Changed);
    let change = facts.summary();
    assert!(change.is_renamed() && change.mode_changed);
    assert_eq!(change.content_changed(), Some(true));
    assert_eq!(change.label(), "renamed, modified");
    assert_eq!(ids(&doc, 0), (Some("aaaaaaa"), Some("bbbbbbb")));
    assert_eq!(hunks(&doc, 0), [(1, 4, 1, 4)]);
    assert_eq!(
        doc.text(0, Side::New).line(1),
        Some("    let label = \"new\";")
    );
}

#[test]
fn binary_and_mode_only_files_are_changes_without_lines() {
    let doc = fixture("binary_mode");
    assert_eq!(doc.file_count(), 2);
    let binary = doc.summary(0);
    assert!(binary.binary);
    assert_eq!(binary.content, ContentChange::Changed);
    assert_eq!(binary.label(), "binary");
    assert!(hunks(&doc, 0).is_empty());
    assert_eq!(ids(&doc, 0), (Some("0123456"), Some("abcdef0")));
    let mode = doc.summary(1);
    assert!(mode.is_mode_only());
    assert_eq!(mode.label(), "mode changed");
    assert_eq!(doc.path(1), "scripts/tool.sh");

    let only = fixture("mode_only");
    let summary = only.summary(0);
    assert_eq!(summary.status, FileStatus::Modified);
    assert_eq!(summary.content, ContentChange::Unchanged);
    assert!(summary.is_mode_only());
    assert_eq!(
        only.stats(0),
        DiffStats::Known {
            additions: 0,
            deletions: 0
        }
    );
}

#[test]
fn missing_final_newlines_survive_parse_and_write() {
    let doc = fixture("no_newline");
    let facts = doc.facts(0);
    assert!(facts.old_missing_newline && facts.new_missing_newline);
    assert_eq!(hunks(&doc, 0), [(1, 1, 1, 1)]);
    let written = write_unified(&doc);
    assert!(written.contains("index 9999999..aaaaaaa 100644\n"));
    assert_eq!(written.matches("\\ No newline at end of file").count(), 2);
    let again = parse_unified(&written).unwrap();
    assert_eq!(again.files().meta, doc.files().meta);
    assert_eq!(hunks(&again, 0), hunks(&doc, 0));
}

#[test]
fn long_lines_stay_whole() {
    let doc = fixture("long_line");
    assert_eq!(hunks(&doc, 0), [(1, 1, 1, 1)]);
    let old = doc.text(0, Side::Old).line(0).unwrap();
    let new = doc.text(0, Side::New).line(0).unwrap();
    assert!(old.len() > 200 && old.starts_with("const app="));
    assert!(new.contains("version:\"1.0.1\""));
}

#[test]
fn crlf_lines_keep_their_carriage_return_for_exact_text() {
    let patch = "diff --git a/w.txt b/w.txt\n\
                 index 1234567..89abcde 100644\n\
                 --- a/w.txt\n\
                 +++ b/w.txt\n\
                 @@ -1,2 +1,2 @@\n \
                 keep\r\n\
                 -old\r\n\
                 +new\r\n";
    let doc = parse_unified(patch).unwrap();
    let new = doc.text(0, Side::New);
    assert_eq!(new.line(1), Some("new\r"));
    assert_eq!(new.display_line(1), Some("new"));
    assert_eq!(hunks(&doc, 0), [(1, 2, 1, 2)]);
}

#[test]
fn invalid_utf8_sources_hydrate_and_map_back_to_bytes() {
    let old = TextStore::from_bytes(b"a\xff\nsame\n");
    let new = TextStore::from_bytes(b"b\xff\nsame\n");
    let doc = diff_from_changes(
        meta("bytes.bin", FileStatus::Modified),
        old,
        new,
        &[Change {
            old: 0..1,
            new: 0..1,
        }],
        3,
    )
    .unwrap();
    let text = doc.text(0, Side::Old);
    assert_eq!(text.display_line(0), Some("a\u{FFFD}"));
    let range = text.line_range(0).unwrap();
    assert_eq!(text.source_bytes(range).as_ref(), b"a\xff");
    assert_eq!(hunks(&doc, 0), [(1, 2, 1, 2)]);
}

fn meta(path: &str, status: FileStatus) -> FileMeta {
    FileMeta {
        old_path: (status != FileStatus::Added).then(|| path.into()),
        new_path: (status != FileStatus::Deleted).then(|| path.into()),
        status,
        ..FileMeta::default()
    }
}

fn span(start: u32, end: u32, intensity: ChangeIntensity) -> SemanticSpan {
    SemanticSpan {
        start,
        end,
        intensity,
    }
}

#[test]
fn semantic_builder_keeps_intensities_and_structural_rows() {
    let old = TextStore::new("fn f() {\n    \"foo\"\n}\nkeep\n");
    let new = TextStore::new("fn f() {\n    \"bar\"\n    extra\n}\nkeep\n");
    let mut builder = DocumentBuilder::new();
    let mut file = builder.file(
        meta("src/lib.rs", FileStatus::Modified),
        SourceCoverage::Full,
        old,
        new,
    );
    file.structural(true)
        .hunk(0, 0)
        .context(1)
        .change(1, 1)
        .change(0, 1)
        .context(1)
        .row(AlignedRow {
            old: Some(0),
            new: Some(0),
        })
        .row(AlignedRow {
            old: Some(1),
            new: Some(1),
        })
        .row(AlignedRow {
            old: None,
            new: Some(2),
        })
        .row(AlignedRow {
            old: Some(2),
            new: Some(3),
        })
        .spans(
            Side::Old,
            1,
            &[
                span(4, 5, ChangeIntensity::UnchangedContext),
                span(5, 8, ChangeIntensity::NovelWord),
                span(8, 9, ChangeIntensity::UnchangedContext),
            ],
        )
        .spans(Side::New, 1, &[span(5, 8, ChangeIntensity::NovelWord)])
        .spans(Side::New, 2, &[span(4, 9, ChangeIntensity::Novel)]);
    assert_eq!(file.finish(), Ok(0));
    let doc = builder.build();
    doc.verify_integrity().unwrap();
    assert!(doc.prefers_structural(0));
    assert_eq!(hunks(&doc, 0), [(1, 3, 1, 4)]);
    assert_eq!(doc.blocks().kind.len(), 3);
    assert_eq!(doc.stats(0).known(), Some((2, 1)));
    let intensities: Vec<_> = doc
        .semantic_spans(0, Side::Old, 1)
        .iter()
        .map(|s| s.intensity)
        .collect();
    assert_eq!(
        intensities,
        [
            ChangeIntensity::UnchangedContext,
            ChangeIntensity::NovelWord,
            ChangeIntensity::UnchangedContext
        ]
    );
    assert_eq!(doc.semantic_spans(0, Side::New, 3), &[]);
    assert_eq!(doc.aligned_rows(0).len(), 4);
    assert_eq!(
        doc.aligned_rows(0)[2],
        AlignedRow {
            old: None,
            new: Some(2)
        }
    );
    let copy = doc.extract_file(0);
    assert_eq!(copy.semantic_spans(0, Side::New, 2).len(), 1);
}

fn build_one(
    coverage: SourceCoverage,
    old: &str,
    new: &str,
    shape: impl FnOnce(&mut diffy_diff::FileBuilder<'_>),
) -> Result<u32, BuildError> {
    let mut builder = DocumentBuilder::new();
    let mut file = builder.file(
        meta("f", FileStatus::Modified),
        coverage,
        TextStore::new(old),
        TextStore::new(new),
    );
    shape(&mut file);
    let result = file.finish();
    if result.is_err() {
        assert_eq!(builder.file_count(), 0);
    }
    result
}

#[test]
fn builder_refuses_inconsistent_documents() {
    let full = SourceCoverage::Full;
    let cases: Vec<(&str, Result<u32, BuildError>, BuildError)> = vec![
        (
            "context differs",
            build_one(full, "a\nb\n", "a\nc\n", |f| {
                f.hunk(0, 0).context(2);
            }),
            BuildError::ContextMismatch {
                old_line: 1,
                new_line: 1,
            },
        ),
        (
            "unequal gaps",
            build_one(full, "a\nb\nc\n", "a\nc\n", |f| {
                f.hunk(2, 1).context(1);
            }),
            BuildError::GapLength {
                old_line: 2,
                new_line: 1,
            },
        ),
        (
            "past the end",
            build_one(full, "a\n", "b\n", |f| {
                f.hunk(0, 0).change(2, 1);
            }),
            BuildError::OutOfRange {
                side: Side::Old,
                line: 1,
            },
        ),
        (
            "lines before a hunk",
            build_one(full, "a\n", "b\n", |f| {
                f.change(1, 1);
            }),
            BuildError::NoHunk,
        ),
        (
            "overlapping hunks",
            build_one(full, "a\nb\n", "x\ny\n", |f| {
                f.hunk(0, 0).change(2, 2).hunk(1, 1).change(1, 1);
            }),
            BuildError::HunkOrder { hunk: 1 },
        ),
        (
            "unused patch lines",
            build_one(SourceCoverage::PatchOnly, "a\nb\n", "c\n", |f| {
                f.hunk(4, 4).change(1, 1);
            }),
            BuildError::UnusedStore(Side::Old),
        ),
        (
            "row outside its hunk",
            build_one(full, "a\nb\n", "a\nc\n", |f| {
                f.hunk(1, 1).change(1, 1).row(AlignedRow {
                    old: Some(0),
                    new: Some(1),
                });
            }),
            BuildError::Semantics(SemanticError::Row { hunk: 0, row: 0 }),
        ),
        (
            "span past the line",
            build_one(full, "a\nb\n", "a\nc\n", |f| {
                f.hunk(1, 1).change(1, 1).spans(
                    Side::New,
                    1,
                    &[span(0, 2, ChangeIntensity::Novel)],
                );
            }),
            BuildError::Semantics(SemanticError::Span {
                side: Side::New,
                line: 1,
            }),
        ),
        (
            "span outside every hunk",
            build_one(full, "a\nb\n", "a\nc\n", |f| {
                f.hunk(1, 1).change(1, 1).spans(
                    Side::New,
                    0,
                    &[span(0, 1, ChangeIntensity::Novel)],
                );
            }),
            BuildError::Semantics(SemanticError::SpanLine {
                side: Side::New,
                line: 0,
            }),
        ),
    ];
    for (name, result, expected) in cases {
        assert_eq!(result, Err(expected), "{name}");
    }
}

#[test]
fn patch_only_builds_map_source_lines_to_stores() {
    let mut builder = DocumentBuilder::new();
    let mut file = builder.file(
        meta("f", FileStatus::Modified),
        SourceCoverage::PatchOnly,
        TextStore::new("ctx\nold\n"),
        TextStore::new("ctx\nnew\n"),
    );
    file.hunk(9, 19)
        .section("fn f()")
        .context(1)
        .change(1, 1)
        .spans(Side::New, 20, &[span(0, 3, ChangeIntensity::Novel)]);
    file.finish().unwrap();
    let doc = builder.build();
    assert_eq!(hunks(&doc, 0), [(10, 2, 20, 2)]);
    assert_eq!(doc.store_line(0, Side::New, 20), Some(1));
    assert_eq!(doc.store_line(0, Side::New, 5), None);
    assert_eq!(doc.coverage(0), SourceCoverage::PatchOnly);
    assert_eq!(doc.hunks().section[0].as_ref(), "fn f()");
}

#[test]
fn line_changes_from_another_algorithm_build_full_documents() {
    let old = TextStore::new("1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n");
    let new = TextStore::new("1\nB\n3\n4\n5\n6\n7\n8\n9\n10\n11\n");
    let doc = diff_from_changes(
        meta("n", FileStatus::Modified),
        old,
        new,
        &[
            Change {
                old: 1..2,
                new: 1..2,
            },
            Change {
                old: 10..10,
                new: 10..11,
            },
        ],
        1,
    )
    .unwrap();
    assert_eq!(hunks(&doc, 0), [(1, 3, 1, 3), (10, 1, 10, 2)]);
    assert_eq!(doc.coverage(0), SourceCoverage::Full);
    assert_eq!(
        diff_from_changes(
            meta("n", FileStatus::Modified),
            TextStore::new("a\n"),
            TextStore::new("b\n"),
            &[Change {
                old: 0..1,
                new: 1..1
            }],
            3,
        )
        .err(),
        Some(BuildError::ChangeOrder { change: 0 })
    );
}

#[test]
fn deferred_files_carry_pending_stats_and_content_facts() {
    assert!(DiffStats::default().is_pending());
    assert_eq!(DiffStats::Pending.known(), None);
    let a = ObjectId::parse("ABCDEF1").unwrap();
    assert_eq!(a.as_str(), "abcdef1");
    assert!(a.matches(&ObjectId::parse("abcdef1234").unwrap()));
    assert_eq!(ObjectId::parse("0000000"), None);
    assert_eq!(ObjectId::parse("xyz"), None);
}

#[test]
fn structural_pairing_follows_aligned_rows_that_cover_the_change() {
    let build = |rows: &[(Option<u32>, Option<u32>)]| {
        let old = TextStore::new("a();\nb();\nc();\n");
        let new = TextStore::new("x();\nb2();\n");
        let mut builder = DocumentBuilder::new();
        let mut file = builder.file(
            meta("f.rs", FileStatus::Modified),
            SourceCoverage::Full,
            old,
            new,
        );
        file.structural(true).hunk(0, 0).change(3, 2);
        for &(old, new) in rows {
            file.row(AlignedRow { old, new });
        }
        let novel = [SemanticSpan {
            start: 0,
            end: 1,
            intensity: ChangeIntensity::Novel,
        }];
        for line in 0..3 {
            file.spans(Side::Old, line, &novel);
        }
        for line in 0..2 {
            file.spans(Side::New, line, &novel);
        }
        file.finish().unwrap();
        builder.build()
    };
    let split = |doc: &DiffDocument, pairing| {
        let comparison = Comparison::new(
            doc,
            ComparisonOptions {
                pairing,
                ..ComparisonOptions::default()
            },
        );
        comparison.verify_integrity(doc).unwrap();
        let expansion = Expansion::new(doc);
        let projection = Projection::with_comparison(doc, Mode::Split, &expansion, &comparison);
        (0..projection.len())
            .filter(|&row| projection.kind[row as usize] != RowKind::FileHeader)
            .map(|row| {
                (
                    projection.line(row, Side::Old),
                    projection.line(row, Side::New),
                )
            })
            .collect::<Vec<_>>()
    };
    let positional = [(Some(0), Some(0)), (Some(1), Some(1)), (Some(2), None)];
    let covered = build(&[
        (Some(0), None),
        (None, Some(0)),
        (Some(1), Some(1)),
        (Some(2), None),
    ]);
    assert_eq!(
        split(&covered, PairingMode::Similarity),
        [
            (Some(0), None),
            (None, Some(0)),
            (Some(1), Some(1)),
            (Some(2), None)
        ]
    );
    assert_eq!(split(&covered, PairingMode::Positional), positional);
    let partial = build(&[(Some(0), None), (Some(1), Some(1)), (Some(2), None)]);
    let plain = Comparison::new(&partial, ComparisonOptions::default());
    let fallback = Comparison::new(&build(&[]), ComparisonOptions::default());
    let rows = |c: &Comparison, doc: &DiffDocument| {
        let expansion = Expansion::new(doc);
        let p = Projection::with_comparison(doc, Mode::Split, &expansion, c);
        (0..p.len())
            .map(|row| (p.line(row, Side::Old), p.line(row, Side::New)))
            .collect::<Vec<_>>()
    };
    assert_eq!(rows(&plain, &partial), rows(&fallback, &build(&[])));
}
