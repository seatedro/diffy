#![no_main]

use carbon::{LineId, TextStore};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: Vec<u8>| {
    let text = TextStore::from_bytes(bytes);
    let mut previous = None;
    for line in 0..text.line_count() {
        let start = text.line_start(LineId(line)).unwrap();
        let range = text.line_range(LineId(line)).unwrap();
        assert_eq!(start, range.start);
        assert!(range.start.saturating_add(range.len) <= text.len());
        if let Some(previous) = previous {
            assert!(previous < start);
        }
        previous = Some(start);
        let _ = text.line_bytes(LineId(line)).unwrap();
        let _ = text.line_str(LineId(line));
    }
});
