use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

use carbon::{
    Block, BlockId, BlockRange, ExpansionDirection, ExpansionState, FileDiff, FileId, Hunk, HunkId,
    ProjectionBuffer, ProjectionOptions, SourceRange, TextStore, expand_context,
    parse_unified_patch, project_file,
};

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);
static ALLOCATION_TEST_LOCK: Mutex<()> = Mutex::new(());

struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        ALLOCATED_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        ALLOCATED_BYTES.fetch_add(new_size, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AllocationSnapshot {
    count: usize,
    bytes: usize,
}

fn reset_allocations() {
    ALLOCATIONS.store(0, Ordering::Relaxed);
    ALLOCATED_BYTES.store(0, Ordering::Relaxed);
}

fn allocations() -> AllocationSnapshot {
    AllocationSnapshot {
        count: ALLOCATIONS.load(Ordering::Relaxed),
        bytes: ALLOCATED_BYTES.load(Ordering::Relaxed),
    }
}

fn allocation_test_lock() -> MutexGuard<'static, ()> {
    ALLOCATION_TEST_LOCK.lock().unwrap()
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

#[derive(Debug, Clone, Copy)]
struct PatchBudget {
    name: &'static str,
    patch: &'static str,
    expected_files: usize,
    max_allocations: usize,
    max_bytes: usize,
}

fn patch_budgets() -> [PatchBudget; 6] {
    [
        PatchBudget {
            name: "many_small_files",
            patch: include_str!("../fixtures/patches/many_small_files.patch"),
            expected_files: 3,
            max_allocations: 500,
            max_bytes: 90_000,
        },
        PatchBudget {
            name: "rename_mode",
            patch: include_str!("../fixtures/patches/rename_mode.patch"),
            expected_files: 1,
            max_allocations: 200,
            max_bytes: 40_000,
        },
        PatchBudget {
            name: "binary_mode",
            patch: include_str!("../fixtures/patches/binary_mode.patch"),
            expected_files: 2,
            max_allocations: 140,
            max_bytes: 30_000,
        },
        PatchBudget {
            name: "long_line",
            patch: include_str!("../fixtures/patches/long_line.patch"),
            expected_files: 1,
            max_allocations: 180,
            max_bytes: 55_000,
        },
        PatchBudget {
            name: "no_newline",
            patch: include_str!("../fixtures/patches/no_newline.patch"),
            expected_files: 1,
            max_allocations: 160,
            max_bytes: 32_000,
        },
        PatchBudget {
            name: "mode_only",
            patch: include_str!("../fixtures/patches/mode_only.patch"),
            expected_files: 1,
            max_allocations: 80,
            max_bytes: 18_000,
        },
    ]
}

#[test]
fn projection_count_emitter_allocates_zero() {
    let _guard = allocation_test_lock();
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

    assert_eq!(count, 1_025);
    assert_eq!(allocations().count, 0);
}

#[test]
fn projection_buffer_rebuild_allocates_zero_after_reserve() {
    let _guard = allocation_test_lock();
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
    let mut buffer = ProjectionBuffer::with_capacity(1_026);
    buffer.rebuild_file(
        &file,
        ProjectionOptions {
            collapsed_context_threshold: 0,
            ..ProjectionOptions::default()
        },
        &expansion,
    );

    reset_allocations();
    buffer.rebuild_file(
        &file,
        ProjectionOptions {
            collapsed_context_threshold: 0,
            ..ProjectionOptions::default()
        },
        &expansion,
    );

    assert_eq!(buffer.len(), 1_025);
    assert_eq!(allocations().count, 0);
}

#[test]
fn patch_parser_stays_under_allocation_budget() {
    let _guard = allocation_test_lock();
    let patch = patch_fixture(8, 16);

    reset_allocations();
    let document = parse_unified_patch(&patch).unwrap();
    let snapshot = allocations();

    assert_eq!(document.files.len(), 8);
    assert!(
        snapshot.count < 1_500,
        "patch parser allocated {} times",
        snapshot.count
    );
    assert!(
        snapshot.bytes < 250_000,
        "patch parser allocated {} bytes",
        snapshot.bytes
    );
}

#[test]
fn patch_fixtures_stay_under_named_allocation_budgets() {
    let _guard = allocation_test_lock();
    for budget in patch_budgets() {
        reset_allocations();
        let document = parse_unified_patch(budget.patch).unwrap();
        let snapshot = allocations();

        assert_eq!(
            document.files.len(),
            budget.expected_files,
            "{} file count",
            budget.name
        );
        assert!(
            snapshot.count <= budget.max_allocations,
            "{} allocated {} times; budget {}",
            budget.name,
            snapshot.count,
            budget.max_allocations
        );
        assert!(
            snapshot.bytes <= budget.max_bytes,
            "{} allocated {} bytes; budget {}",
            budget.name,
            snapshot.bytes,
            budget.max_bytes
        );
    }
}
