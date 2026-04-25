use carbon::{
    Block, BlockId, BlockRange, ExpansionDirection, ExpansionState, FileDiff, FileId, Hunk, HunkId,
    InlineDiffMode, InlineOptions, ProjectionMode, ProjectionOptions, ProjectionRowKind,
    ProjectionWindow, SourceRange, TextStore, compute_inline_diff, expand_context, project_file,
    project_window,
};
use proptest::prelude::*;

fn text_lines(lines: &[String]) -> String {
    let mut text = String::new();
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    text
}

fn changed_file(prefix: &[String], old_line: &str, new_line: &str, suffix: &[String]) -> FileDiff {
    let mut old_lines = prefix.to_vec();
    old_lines.push(old_line.to_owned());
    old_lines.extend_from_slice(suffix);

    let mut new_lines = prefix.to_vec();
    new_lines.push(new_line.to_owned());
    new_lines.extend_from_slice(suffix);

    let change_index = prefix.len().min(u32::MAX as usize) as u32;
    let mut file = FileDiff {
        id: FileId(1),
        old_text: Some(TextStore::from_text(text_lines(&old_lines))),
        new_text: Some(TextStore::from_text(text_lines(&new_lines))),
        ..FileDiff::default()
    };
    file.add_hunk(
        Hunk::new(
            HunkId(0),
            change_index + 1,
            1,
            change_index + 1,
            1,
            BlockRange::default(),
        ),
        [Block::change(
            BlockId(0),
            SourceRange::new(change_index, 1),
            SourceRange::new(change_index, 1),
        )],
    );
    file
}

fn line_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_]{0,24}".prop_map(String::from)
}

proptest! {
    #[test]
    fn text_store_ranges_are_monotonic_and_in_bounds(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let text = TextStore::from_bytes(bytes);
        let mut previous = None;
        for line in 0..text.line_count() {
            let start = text.line_start(carbon::LineId(line)).unwrap();
            let range = text.line_range(carbon::LineId(line)).unwrap();
            prop_assert_eq!(start, range.start);
            prop_assert!(range.start.saturating_add(range.len) <= text.len());
            if let Some(previous) = previous {
                prop_assert!(previous < start);
            }
            previous = Some(start);
        }
    }

    #[test]
    fn projection_windows_reconstruct_full_projection(
        prefix in proptest::collection::vec(line_strategy(), 0..16),
        old_line in line_strategy(),
        new_line in line_strategy(),
        suffix in proptest::collection::vec(line_strategy(), 0..16),
        mode in prop_oneof![
            Just(ProjectionMode::Unified),
            Just(ProjectionMode::Split),
            Just(ProjectionMode::Both),
        ],
        threshold in 0_u32..4,
        window_len in 1_u32..8,
    ) {
        let file = changed_file(&prefix, &old_line, &new_line, &suffix);
        let options = ProjectionOptions {
            mode,
            collapsed_context_threshold: threshold,
            ..ProjectionOptions::default()
        };
        let mut full = Vec::new();
        project_file(&file, options, &ExpansionState::default(), |row| full.push(row));

        let mut windowed = Vec::new();
        let mut start = 0;
        while start < full.len() as u32 {
            project_window(
                &file,
                options,
                &ExpansionState::default(),
                ProjectionWindow { start, len: window_len },
                |row| windowed.push(row),
            );
            start = start.saturating_add(window_len);
        }

        prop_assert_eq!(windowed, full);
    }

    #[test]
    fn context_expansion_never_changes_change_rows(
        prefix in proptest::collection::vec(line_strategy(), 0..16),
        old_line in line_strategy(),
        new_line in line_strategy(),
        suffix in proptest::collection::vec(line_strategy(), 0..16),
        above in 0_u32..32,
        below in 0_u32..32,
    ) {
        let file = changed_file(&prefix, &old_line, &new_line, &suffix);
        let options = ProjectionOptions {
            collapsed_context_threshold: 0,
            ..ProjectionOptions::default()
        };
        let mut baseline = Vec::new();
        project_file(&file, options, &ExpansionState::default(), |row| baseline.push(row));
        let baseline_changes = baseline
            .iter()
            .copied()
            .filter(|row| matches!(
                row.kind,
                ProjectionRowKind::Added | ProjectionRowKind::Removed | ProjectionRowKind::Modified
            ))
            .collect::<Vec<_>>();
        let baseline_change_count = baseline_changes.len();

        let mut expansion = ExpansionState::default();
        expand_context(&file, &mut expansion, HunkId(0), ExpansionDirection::Above, above);
        expand_context(&file, &mut expansion, HunkId(0), ExpansionDirection::Below, below);
        let mut expanded = Vec::new();
        project_file(&file, options, &expansion, |row| expanded.push(row));
        let expanded_changes = expanded
            .iter()
            .copied()
            .filter(|row| matches!(
                row.kind,
                ProjectionRowKind::Added | ProjectionRowKind::Removed | ProjectionRowKind::Modified
            ))
            .collect::<Vec<_>>();

        prop_assert_eq!(expanded_changes, baseline_changes);
        prop_assert!(expanded.len() >= baseline_change_count);
    }

    #[test]
    fn inline_diff_spans_stay_sorted_and_in_bounds(
        old in ".{0,128}",
        new in ".{0,128}",
        mode in prop_oneof![
            Just(InlineDiffMode::Word),
            Just(InlineDiffMode::WordAlt),
            Just(InlineDiffMode::Char),
            Just(InlineDiffMode::None),
        ],
    ) {
        let diff = compute_inline_diff(
            &old,
            &new,
            InlineOptions {
                mode,
                max_line_len: 512,
            },
        );
        for (text, spans) in [(&old, &diff.old), (&new, &diff.new)] {
            let mut previous_end = 0;
            for span in spans {
                let end = span.offset.saturating_add(span.len);
                prop_assert!(span.len > 0);
                prop_assert!(end as usize <= text.len());
                prop_assert!(span.offset >= previous_end);
                prop_assert!(text.is_char_boundary(span.offset as usize));
                prop_assert!(text.is_char_boundary(end as usize));
                previous_end = end;
            }
        }
        if mode == InlineDiffMode::None {
            prop_assert!(diff.old.is_empty());
            prop_assert!(diff.new.is_empty());
        }
    }
}
