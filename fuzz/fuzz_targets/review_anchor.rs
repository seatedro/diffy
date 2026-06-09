#![no_main]

use carbon::{
    Anchor, DiffSide, ExpansionState, LineRange, ProjectionOptions, map_anchor_to_projection,
    parse_unified_patch, project_file, projected_row_byte_range,
};
use libfuzzer_sys::fuzz_target;

// Round-trips arbitrary review anchors against projected rows and checks that
// row-to-text byte ranges stay inside the side text stores.
fuzz_target!(|data: (&str, u8, u32, u32)| {
    let (input, side_byte, start, len) = data;
    let Ok(document) = parse_unified_patch(input) else {
        return;
    };

    let side = match side_byte % 3 {
        0 => Some(DiffSide::Old),
        1 => Some(DiffSide::New),
        _ => None,
    };

    for file in &document.files {
        let mut rows = Vec::new();
        project_file(
            file,
            ProjectionOptions::default(),
            &ExpansionState::default(),
            |row| rows.push(row),
        );

        let anchor = Anchor {
            side,
            line_range: LineRange::new(start, len),
            ..Anchor::file(file.id)
        };
        let touched = map_anchor_to_projection(&anchor, &rows);
        let expected = rows.iter().filter(|row| anchor.touches_row(row)).count();
        assert_eq!(touched.len(), expected);
        for row in &touched {
            assert!(anchor.touches_row(row));
        }

        for row in &rows {
            for side in [DiffSide::Old, DiffSide::New] {
                let Some(range) = projected_row_byte_range(file, row, side) else {
                    continue;
                };
                let text = file
                    .side_text(side)
                    .expect("a projected byte range implies side text exists");
                assert!(range.start.saturating_add(range.len) <= text.len());
            }
        }
    }
});
