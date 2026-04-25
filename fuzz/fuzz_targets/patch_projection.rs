#![no_main]

use carbon::{
    ExpansionState, ProjectionMode, ProjectionOptions, ProjectionRowKind, parse_unified_patch,
    project_file,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &str| {
    let Ok(document) = parse_unified_patch(input) else {
        return;
    };

    for file in &document.files {
        for mode in [
            ProjectionMode::Unified,
            ProjectionMode::Split,
            ProjectionMode::Both,
        ] {
            let mut rows = Vec::new();
            project_file(
                file,
                ProjectionOptions {
                    mode,
                    collapsed_context_threshold: 2,
                    ..ProjectionOptions::default()
                },
                &ExpansionState::default(),
                |row| rows.push(row),
            );
            for row in rows {
                assert_eq!(row.file_id, file.id);
                if row.kind == ProjectionRowKind::ContextGap {
                    assert!(row.collapsed_count > 0);
                }
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
            }
        }
    }
});
