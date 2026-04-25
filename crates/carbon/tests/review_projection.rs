use carbon::{
    Anchor, Annotation, AnnotationId, AnnotationKind, AnnotationSet, DiffSide, ExpansionState,
    FileDiff, LineRange, ProjectionMode, ProjectionOptions, ProjectionRow, ProjectionRowKind,
    parse_unified_patch, project_file,
};

fn fixture_file() -> FileDiff {
    let document =
        parse_unified_patch(include_str!("../fixtures/patches/many_small_files.patch")).unwrap();
    document.files[0].clone()
}

fn project(file: &FileDiff, mode: ProjectionMode) -> Vec<ProjectionRow> {
    let mut rows = Vec::new();
    project_file(
        file,
        ProjectionOptions {
            mode,
            include_hunk_headers: false,
            collapsed_context_threshold: u32::MAX,
        },
        &ExpansionState::default(),
        |row| rows.push(row),
    );
    rows
}

fn annotation_ids_for_row(annotations: &AnnotationSet, row: &ProjectionRow) -> Vec<AnnotationId> {
    annotations
        .for_row(row)
        .map(|annotation| annotation.id)
        .collect()
}

fn comment(id: u64, anchor: Anchor, message: &str) -> Annotation {
    Annotation {
        id: AnnotationId(id),
        anchor,
        kind: AnnotationKind::Comment,
        message: message.to_owned(),
    }
}

#[test]
fn review_anchors_attach_to_old_new_both_and_file_level_rows() {
    let file = fixture_file();
    let rows = project(&file, ProjectionMode::Unified);
    let mut annotations = AnnotationSet::new();
    annotations.push(comment(
        1,
        Anchor {
            file_id: file.id,
            side: Some(DiffSide::Old),
            line_range: LineRange::new(2, 1),
            byte_range: None,
            old_oid: file.old_oid.clone(),
            new_oid: file.new_oid.clone(),
        },
        "old-side removed line",
    ));
    annotations.push(comment(
        2,
        Anchor {
            file_id: file.id,
            side: Some(DiffSide::New),
            line_range: LineRange::new(2, 1),
            byte_range: None,
            old_oid: file.old_oid.clone(),
            new_oid: file.new_oid.clone(),
        },
        "new-side added line",
    ));
    annotations.push(comment(
        3,
        Anchor {
            file_id: file.id,
            side: None,
            line_range: LineRange::new(1, 3),
            byte_range: None,
            old_oid: file.old_oid.clone(),
            new_oid: file.new_oid.clone(),
        },
        "both sides",
    ));
    annotations.push(comment(4, Anchor::file(file.id), "file-level"));

    let removed = rows
        .iter()
        .find(|row| row.kind == ProjectionRowKind::Removed)
        .unwrap();
    let added = rows
        .iter()
        .find(|row| row.kind == ProjectionRowKind::Added)
        .unwrap();

    assert_eq!(
        annotation_ids_for_row(&annotations, removed),
        vec![AnnotationId(1), AnnotationId(3), AnnotationId(4)]
    );
    assert_eq!(
        annotation_ids_for_row(&annotations, added),
        vec![AnnotationId(2), AnnotationId(3), AnnotationId(4)]
    );
    assert!(rows.iter().all(|row| {
        annotations
            .for_row(row)
            .any(|annotation| annotation.id == AnnotationId(4))
    }));
}

#[test]
fn side_specific_annotations_survive_unified_and_split_projection() {
    let file = fixture_file();
    let mut annotations = AnnotationSet::new();
    annotations.push(comment(
        1,
        Anchor {
            file_id: file.id,
            side: Some(DiffSide::Old),
            line_range: LineRange::new(2, 1),
            byte_range: None,
            old_oid: file.old_oid.clone(),
            new_oid: file.new_oid.clone(),
        },
        "old-side removed line",
    ));
    annotations.push(comment(
        2,
        Anchor {
            file_id: file.id,
            side: Some(DiffSide::New),
            line_range: LineRange::new(2, 1),
            byte_range: None,
            old_oid: file.old_oid.clone(),
            new_oid: file.new_oid.clone(),
        },
        "new-side added line",
    ));

    for mode in [ProjectionMode::Unified, ProjectionMode::Split] {
        let rows = project(&file, mode);
        let old_hits = rows
            .iter()
            .filter(|row| {
                annotations
                    .for_row(row)
                    .any(|annotation| annotation.id == AnnotationId(1))
            })
            .collect::<Vec<_>>();
        let new_hits = rows
            .iter()
            .filter(|row| {
                annotations
                    .for_row(row)
                    .any(|annotation| annotation.id == AnnotationId(2))
            })
            .collect::<Vec<_>>();

        assert_eq!(old_hits.len(), 1, "old-side hit count in {mode:?}");
        assert_eq!(new_hits.len(), 1, "new-side hit count in {mode:?}");
        assert_eq!(old_hits[0].old_line, Some(2));
        assert_eq!(new_hits[0].new_line, Some(2));
    }
}
