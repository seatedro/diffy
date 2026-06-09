#![no_main]

use carbon::{BlockId, FileId, HunkId, parse_unified_patch, usize_to_u32_saturating};
use libfuzzer_sys::fuzz_target;

// Exercises the parser's error paths on arbitrary (possibly non-UTF-8) bytes
// and checks structural invariants of any document it accepts.
fuzz_target!(|bytes: &[u8]| {
    let input = String::from_utf8_lossy(bytes);
    let Ok(document) = parse_unified_patch(&input) else {
        return;
    };

    assert!(!document.files.is_empty());
    for (file_index, file) in document.files.iter().enumerate() {
        assert_eq!(file.id, FileId(usize_to_u32_saturating(file_index)));
        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            assert_eq!(hunk.id, HunkId(usize_to_u32_saturating(hunk_index)));
            assert!(file.hunk(hunk.id).is_some());
            assert!(hunk.blocks.end() as usize <= file.blocks.len());
            assert_eq!(file.hunk_blocks(hunk).len() as u32, hunk.blocks.len);
            assert!(hunk.old_start_index() <= hunk.old_end_index());
            assert!(hunk.new_start_index() <= hunk.new_end_index());
        }
        for (block_index, block) in file.blocks.iter().enumerate() {
            assert_eq!(block.id, BlockId(usize_to_u32_saturating(block_index)));
            assert!(file.block(block.id).is_some());
            if block.old.len > 0 {
                let text = file.old_text.as_ref();
                assert!(text.is_some_and(|text| block.old.end() <= text.line_count()));
            }
            if block.new.len > 0 {
                let text = file.new_text.as_ref();
                assert!(text.is_some_and(|text| block.new.end() <= text.line_count()));
            }
        }
    }
});
