#![no_main]

use diffy_diff::TextStore;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let text = TextStore::from_bytes(bytes);
    assert_eq!(text.to_source_bytes().as_ref(), bytes);
    let mut previous_end = 0;
    for line in 0..text.line_count() {
        let range = text.line_range(line).unwrap();
        assert!(range.start >= previous_end);
        assert!(range.end <= text.as_str().len());
        assert!(text.as_str().is_char_boundary(range.start));
        assert!(text.as_str().is_char_boundary(range.end));
        previous_end = range.end;
        let _ = text.line(line).unwrap();
        let _ = text.display_line(line);
    }
});
