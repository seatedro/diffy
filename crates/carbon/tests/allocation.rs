use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use carbon::{
    Block, BlockId, BlockRange, ExpansionDirection, ExpansionState, FileDiff, FileId, Hunk, HunkId,
    ProjectionOptions, SourceRange, TextStore, expand_context, parse_unified_patch, project_file,
};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

fn reset_allocations() {
    ALLOCATIONS.store(0, Ordering::Relaxed);
}

fn allocations() -> usize {
    ALLOCATIONS.load(Ordering::Relaxed)
}

fn large_file() -> FileDiff {
    let mut old = String::new();
    let mut new = String::new();
    for index in 0..1_024 {
        old.push_str("line ");
        old.push_str(&index.to_string());
        old.push('\n');

        if index == 512 {
            new.push_str("changed\n");
        } else {
            new.push_str("line ");
            new.push_str(&index.to_string());
            new.push('\n');
        }
    }

    let mut file = FileDiff {
        id: FileId(1),
        old_text: Some(TextStore::from_text(old)),
        new_text: Some(TextStore::from_text(new)),
        ..FileDiff::default()
    };
    file.add_hunk(
        Hunk::new(HunkId(0), 513, 1, 513, 1, BlockRange::default()),
        [Block::change(
            BlockId(0),
            SourceRange::new(512, 1),
            SourceRange::new(512, 1),
        )],
    );
    file
}

fn patch_fixture(file_count: usize, lines_per_file: usize) -> String {
    let mut patch = String::new();
    for file_index in 0..file_count {
        patch.push_str("diff --git a/file");
        patch.push_str(&file_index.to_string());
        patch.push_str(".rs b/file");
        patch.push_str(&file_index.to_string());
        patch.push_str(".rs\nindex 1111111..2222222 100644\n--- a/file");
        patch.push_str(&file_index.to_string());
        patch.push_str(".rs\n+++ b/file");
        patch.push_str(&file_index.to_string());
        patch.push_str(".rs\n@@ -1,");
        patch.push_str(&lines_per_file.to_string());
        patch.push_str(" +1,");
        patch.push_str(&lines_per_file.to_string());
        patch.push_str(" @@\n");
        for line in 0..lines_per_file {
            if line == lines_per_file / 2 {
                patch.push_str("-let value = 1;\n+let value = 2;\n");
            } else {
                patch.push_str(" let value = 0;\n");
            }
        }
    }
    patch
}

#[test]
fn projection_count_emitter_allocates_zero() {
    let file = large_file();
    let mut expansion = ExpansionState::default();
    expand_context(
        &file,
        &mut expansion,
        HunkId(0),
        ExpansionDirection::Above,
        u32::MAX,
    );
    expand_context(
        &file,
        &mut expansion,
        HunkId(0),
        ExpansionDirection::Below,
        u32::MAX,
    );

    reset_allocations();
    let mut count = 0;
    project_file(
        &file,
        ProjectionOptions {
            collapsed_context_threshold: 0,
            ..ProjectionOptions::default()
        },
        &expansion,
        |row| {
            std::hint::black_box(row);
            count += 1;
        },
    );

    assert_eq!(count, 1_026);
    assert_eq!(allocations(), 0);
}

#[test]
fn patch_parser_stays_under_allocation_budget() {
    let patch = patch_fixture(8, 16);

    reset_allocations();
    let document = parse_unified_patch(&patch).unwrap();
    let allocation_count = allocations();

    assert_eq!(document.files.len(), 8);
    assert!(
        allocation_count < 1_500,
        "patch parser allocated {allocation_count} times"
    );
}
