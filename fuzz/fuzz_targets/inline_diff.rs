#![no_main]

use diffy_diff::{InlineMode, InlineOptions, paired_inline_diff};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: (&str, &str, u8)| {
    let (old, new, mode_byte) = data;
    let mode = match mode_byte % 3 {
        0 => InlineMode::Word,
        1 => InlineMode::Grapheme,
        _ => InlineMode::Off,
    };
    let diff = paired_inline_diff(
        old,
        new,
        &InlineOptions {
            mode,
            max_line_bytes: 512,
            ..InlineOptions::default()
        },
    );

    for (text, ranges) in [(old, &diff.old), (new, &diff.new)] {
        let mut previous_end = 0;
        for range in ranges {
            assert!(range.start < range.end);
            assert!(range.start >= previous_end);
            assert!(range.end as usize <= text.len());
            assert!(text.is_char_boundary(range.start as usize));
            assert!(text.is_char_boundary(range.end as usize));
            previous_end = range.end;
        }
    }
});
