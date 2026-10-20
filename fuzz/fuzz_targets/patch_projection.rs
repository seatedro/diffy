#![no_main]

use diffy_diff::{Expansion, Mode, Projection, Side, parse_unified};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &str| {
    let Ok(document) = parse_unified(input) else {
        return;
    };
    let expansion = Expansion::new(&document);
    for mode in [Mode::Unified, Mode::Split] {
        let projection = Projection::new(&document, mode, &expansion);
        projection.verify_integrity(&document).unwrap();
        for row in 0..projection.len() {
            let file = projection.file[row as usize];
            for side in [Side::Old, Side::New] {
                let Some(line) = projection.line(row, side) else {
                    continue;
                };
                assert!(line < document.text(file, side).line_count());
            }
        }
    }
});
