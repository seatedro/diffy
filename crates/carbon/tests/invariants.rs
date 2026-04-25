use carbon::{
    Anchor, Annotation, AnnotationId, AnnotationKind, AnnotationSet, Block, BlockId, BlockRange,
    ByteRange, DiffSide, ExpansionState, FileDiff, FileId, Hunk, HunkId, InlineDiffMode,
    InlineOptions, LineId, LineRange, ProjectionMode, ProjectionOptions, ProjectionRow,
    ProjectionRowKind, ProjectionWindow, SourceRange, TextStore, compute_inline_diff, project_file,
    project_window,
};

fn assert_text_store_invariants(text: &TextStore) {
    let mut previous_start = None;
    for line in 0..text.line_count() {
        let start = text.line_start(LineId(line)).unwrap();
        let range = text.line_range(LineId(line)).unwrap();
        assert!(start <= text.len());
        assert_eq!(range.start, start);
        assert!(range.start.saturating_add(range.len) <= text.len());
        if let Some(previous) = previous_start {
            assert!(previous < start);
        }
        previous_start = Some(start);
        assert_eq!(
            text.line_bytes(LineId(line)).unwrap().len(),
            range.len as usize
        );
    }
    assert_eq!(
        text.has_trailing_newline(),
        text.as_bytes().last() == Some(&b'\n')
    );
    assert_eq!(
        text.no_newline_at_eof(),
        !text.is_empty() && !text.has_trailing_newline()
    );
}

fn assert_projection_rows_invariants(file: &FileDiff, rows: &[ProjectionRow]) {
    for row in rows {
        assert_eq!(row.file_id, file.id);
        if let Some(index) = row.old_index {
            assert!(
                file.old_text
                    .as_ref()
                    .is_some_and(|text| index < text.line_count())
            );
        }
        if let Some(index) = row.new_index {
            assert!(
                file.new_text
                    .as_ref()
                    .is_some_and(|text| index < text.line_count())
            );
        }
        if row.kind == ProjectionRowKind::ContextGap {
            assert!(row.collapsed_count > 0);
            assert!(row.old_index.is_none());
            assert!(row.new_index.is_none());
        } else {
            assert_eq!(row.collapsed_count, 0);
        }
    }
}

fn assert_inline_spans_invariants(text: &str, spans: &[carbon::InlineSpan]) {
    let mut previous_end = 0;
    for span in spans {
        let end = span.offset.saturating_add(span.len);
        assert!(span.len > 0);
        assert!(end as usize <= text.len());
        assert!(span.offset >= previous_end);
        assert!(text.is_char_boundary(span.offset as usize));
        assert!(text.is_char_boundary(end as usize));
        previous_end = end;
    }
}

fn sample_full_file() -> FileDiff {
    let mut file = FileDiff {
        id: FileId(7),
        old_path: Some("src/lib.rs".to_owned()),
        new_path: Some("src/lib.rs".to_owned()),
        old_text: Some(TextStore::from_text("a\nb\nold\nd\ne\nf\ng\n")),
        new_text: Some(TextStore::from_text("a\nb\nnew\nd\ne\nf\ng\n")),
        ..FileDiff::default()
    };
    file.add_hunk(
        Hunk::new(HunkId(0), 3, 1, 3, 1, BlockRange::default()),
        [Block::change(
            BlockId(0),
            SourceRange::new(2, 1),
            SourceRange::new(2, 1),
        )],
    );
    file
}

#[test]
fn text_store_exposes_sorted_in_bounds_line_ranges() {
    for text in [
        "",
        "\n",
        "a",
        "a\n",
        "a\r\nb",
        "α\nβ\nγ",
        "long line without final newline",
    ] {
        assert_text_store_invariants(&TextStore::from_text(text));
    }
}

#[test]
fn projection_rows_are_in_bounds_for_all_modes() {
    let file = sample_full_file();
    for mode in [
        ProjectionMode::Unified,
        ProjectionMode::Split,
        ProjectionMode::Both,
    ] {
        let mut rows = Vec::new();
        project_file(
            &file,
            ProjectionOptions {
                mode,
                collapsed_context_threshold: 0,
                ..ProjectionOptions::default()
            },
            &ExpansionState::default(),
            |row| rows.push(row),
        );
        assert_projection_rows_invariants(&file, &rows);
    }
}

#[test]
fn projection_windows_concatenate_to_full_projection() {
    let file = sample_full_file();
    let options = ProjectionOptions {
        collapsed_context_threshold: 0,
        ..ProjectionOptions::default()
    };
    let mut full = Vec::new();
    project_file(&file, options, &ExpansionState::default(), |row| {
        full.push(row)
    });

    let mut windowed = Vec::new();
    for start in 0..full.len() as u32 {
        project_window(
            &file,
            options,
            &ExpansionState::default(),
            ProjectionWindow { start, len: 1 },
            |row| windowed.push(row),
        );
    }

    assert_eq!(windowed, full);
}

#[test]
fn inline_spans_are_sorted_and_in_bounds() {
    let pairs = [
        ("let old = 1;", "let new = 1;"),
        ("abc", "axc"),
        ("same", "same"),
        ("héllo world", "hello brave world"),
    ];
    for (old, new) in pairs {
        for mode in [
            InlineDiffMode::Word,
            InlineDiffMode::WordAlt,
            InlineDiffMode::Char,
        ] {
            let diff = compute_inline_diff(
                old,
                new,
                InlineOptions {
                    mode,
                    max_line_len: 10_000,
                },
            );
            assert_inline_spans_invariants(old, &diff.old);
            assert_inline_spans_invariants(new, &diff.new);
        }
    }
}

#[test]
fn annotations_map_to_projection_rows_by_side_and_file() {
    let file = sample_full_file();
    let mut rows = Vec::new();
    project_file(
        &file,
        ProjectionOptions::default(),
        &ExpansionState::default(),
        |row| rows.push(row),
    );

    let mut annotations = AnnotationSet::new();
    annotations.push(Annotation {
        id: AnnotationId(1),
        anchor: Anchor {
            file_id: file.id,
            side: Some(DiffSide::New),
            line_range: LineRange::new(3, 1),
            byte_range: Some(ByteRange { start: 0, len: 3 }),
            old_oid: None,
            new_oid: None,
        },
        kind: AnnotationKind::Comment,
        message: "review note".to_owned(),
    });

    let touched = rows
        .iter()
        .filter(|row| annotations.for_row(row).next().is_some())
        .collect::<Vec<_>>();
    assert_eq!(touched.len(), 1);
    assert_eq!(touched[0].kind, ProjectionRowKind::Added);
    assert_eq!(touched[0].new_line, Some(3));
}
