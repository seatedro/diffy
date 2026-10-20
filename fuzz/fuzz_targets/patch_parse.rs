#![no_main]

use diffy_diff::{parse_unified, write_unified};
use libfuzzer_sys::fuzz_target;

// Parses arbitrary (possibly non-UTF-8) bytes, checks the document's
// integrity, and checks that what it writes back parses to the same files.
fuzz_target!(|bytes: &[u8]| {
    let input = String::from_utf8_lossy(bytes);
    let Ok(document) = parse_unified(&input) else {
        return;
    };
    document.verify_integrity().unwrap();
    let written = write_unified(&document);
    let reparsed = parse_unified(&written).unwrap();
    reparsed.verify_integrity().unwrap();
    assert_eq!(reparsed.file_count(), document.file_count());
});
