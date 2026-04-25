#![no_main]

use carbon::{InlineDiffMode, InlineOptions, compute_inline_diff};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: (&str, &str, u8)| {
    let (old, new, mode_byte) = data;
    let mode = match mode_byte % 4 {
        0 => InlineDiffMode::Word,
        1 => InlineDiffMode::WordAlt,
        2 => InlineDiffMode::Char,
        _ => InlineDiffMode::None,
    };
    let diff = compute_inline_diff(
        old,
        new,
        InlineOptions {
            mode,
            max_line_len: 512,
        },
    );

    for (text, spans) in [(old, &diff.old), (new, &diff.new)] {
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
});
